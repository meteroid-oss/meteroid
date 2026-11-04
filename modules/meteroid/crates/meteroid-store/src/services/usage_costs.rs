use crate::StoreResult;
use crate::domain::billable_metrics::SegmentationMatrix;
use crate::domain::credit_outlook::UpcomingChange;
use crate::domain::enums::SubscriptionStatusEnum;
use crate::domain::usage_costs::{
    CostGranularity, CostUnit, MetricEventCost, MetricEventsPage, MetricRef, MetricRunningCost,
    RunningTotal, UsageCost, UsageCostRow, UsageOwner, WalletDailyUsage, WalletDailyUsageCost,
    WalletUsageSpan,
};
use crate::domain::{
    BillableMetric, LineItem, SubscriptionDetails, UsagePeriod, UsagePricingModel,
};
use crate::errors::StoreError;
use crate::repositories::billable_metrics::BillableMetricInterface;
use crate::repositories::credit::customer_credit::normalized_currency;
use crate::repositories::subscriptions::SubscriptionInterface;
use crate::services::ServicesEdge;
use crate::services::clients::usage::{
    GroupedUsageData, MetricEvent, MetricEventCursor, TenantUsageGrain, UsageSeriesPoint,
};
use crate::services::credit::billing_windows::{PlannedWindow, price_window_groups, windows_at};
use crate::services::entitlements::USAGE_FETCH_CONCURRENCY;
use crate::services::invoice_lines::component::{
    PinnedPrices, is_usage_based_line, matrix_cell_matches, rounds_converted_units, unrounded_units,
};
use crate::services::invoice_lines::invoice_lines::{BilledSpan, MeteredFee};
use crate::services::invoices::upcoming::upcoming_invoice_details_on;
use crate::services::usage_grouping::{foldable, metric_dimensions};
use crate::store::PgConn;
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc};
use common_domain::ids::{BaseId, CustomerId, SubscriptionId, TenantId, WalletId};
use diesel_models::customer_credit::CustomerCreditAccountRow;
use diesel_models::query::credit_usage::{
    ConsolidatedDrafts, RecurringInvoiceCycle, list_draft_recurring_periods,
    list_recurring_invoice_cycles,
};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use futures::stream::{self, StreamExt, TryStreamExt};
use rust_decimal::Decimal;
use std::cmp::Reverse;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};

const MAX_RANGE: Duration = Duration::days(93);
const MAX_BILLING_PERIOD: Duration = Duration::days(366);
pub const MAX_EVENTS_PAGE: u32 = 500;
const MAX_GROUP_BY_LEN: usize = 128;
/// A billing period is at most a year; the guard only stops a malformed window.
const MAX_WINDOW_DAYS: usize = 400;

fn invalid(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.into()))
}

fn validate_range(from: NaiveDateTime, to: NaiveDateTime) -> StoreResult<()> {
    if from >= to {
        return Err(invalid("`from` must be before `to`"));
    }
    if to - from > MAX_RANGE {
        return Err(invalid("the range cannot exceed 93 days"));
    }
    Ok(())
}

fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

fn next_month(date: NaiveDate) -> NaiveDate {
    let first = month_start(date);
    first
        .checked_add_months(chrono::Months::new(1))
        .unwrap_or(first)
}

fn bucket_floor(ts: NaiveDateTime, granularity: CostGranularity) -> NaiveDateTime {
    match granularity {
        CostGranularity::Hour => ts
            .date()
            .and_time(NaiveTime::from_hms_opt(ts.hour(), 0, 0).unwrap_or(NaiveTime::MIN)),
        CostGranularity::Day => ts.date().and_time(NaiveTime::MIN),
        CostGranularity::Month => month_start(ts.date()).and_time(NaiveTime::MIN),
    }
}

fn bucket_end(start: NaiveDateTime, granularity: CostGranularity) -> NaiveDateTime {
    match granularity {
        CostGranularity::Hour => start + Duration::hours(1),
        CostGranularity::Day => start + Duration::days(1),
        CostGranularity::Month => next_month(start.date()).and_time(NaiveTime::MIN),
    }
}

fn bucket_ceil(ts: NaiveDateTime, granularity: CostGranularity) -> NaiveDateTime {
    let floor = bucket_floor(ts, granularity);
    if floor == ts {
        ts
    } else {
        bucket_end(floor, granularity)
    }
}

fn segmentation_keys(metric: &BillableMetric) -> Vec<&str> {
    match &metric.segmentation_matrix {
        Some(SegmentationMatrix::Single(dimension)) => vec![dimension.key.as_str()],
        Some(SegmentationMatrix::Double {
            dimension1,
            dimension2,
        }) => vec![dimension1.key.as_str(), dimension2.key.as_str()],
        Some(SegmentationMatrix::Linked {
            dimension1_key,
            dimension2_key,
            ..
        }) => vec![dimension1_key.as_str(), dimension2_key.as_str()],
        None => vec![],
    }
}

/// Whether the metric's segmentation lets an event with these properties through, as the
/// metering query applies it.
fn segmentation_admits(metric: &BillableMetric, properties: &HashMap<String, String>) -> bool {
    let has = |key: &str, values: &[String]| {
        properties
            .get(key)
            .is_some_and(|value| values.iter().any(|allowed| allowed == value))
    };
    match &metric.segmentation_matrix {
        None => true,
        Some(SegmentationMatrix::Single(dimension)) => has(&dimension.key, &dimension.values),
        Some(SegmentationMatrix::Double {
            dimension1,
            dimension2,
        }) => has(&dimension1.key, &dimension1.values) && has(&dimension2.key, &dimension2.values),
        Some(SegmentationMatrix::Linked {
            dimension1_key,
            dimension2_key,
            values,
        }) => properties
            .get(dimension1_key)
            .and_then(|dimension1| values.get(dimension1))
            .is_some_and(|linked| linked.is_empty() || has(dimension2_key, linked)),
    }
}

fn parse_quantity(raw: &str) -> Option<Decimal> {
    raw.parse::<Decimal>()
        .ok()
        .or_else(|| Decimal::from_scientific(raw).ok())
}

/// The units the event adds to the metric, before unit conversion.
fn event_units(metric: &BillableMetric, event: &MetricEvent) -> Option<Decimal> {
    use crate::domain::enums::BillingMetricAggregateEnum as Aggregate;
    if !foldable(&metric.aggregation_type) || !segmentation_admits(metric, &event.properties) {
        return None;
    }
    match metric.aggregation_type {
        Aggregate::Count => Some(Decimal::ONE),
        Aggregate::Sum => metric
            .aggregation_key
            .as_ref()
            .and_then(|key| event.properties.get(key))
            .and_then(|raw| parse_quantity(raw)),
        _ => None,
    }
}

fn billed_units(metric: &BillableMetric, units: Decimal) -> Decimal {
    unrounded_units(metric.unit_conversion_factor, units)
}

fn event_cost(
    window: &PlannedWindow,
    pinned: &PinnedPrices,
    event: &MetricEvent,
    quantity: Decimal,
) -> Option<UsageCost> {
    let MeteredFee::Usage(model) = &window.fee else {
        return None;
    };
    // A rounded conversion prices the period total, so one event has no price of its own.
    if rounds_converted_units(&window.metric) {
        return None;
    }
    let amount = match model {
        UsagePricingModel::PerUnit { rate } => {
            let dimensions = metric_dimensions(&window.metric)
                .into_iter()
                .filter_map(|key| {
                    let value = event.properties.get(&key)?.clone();
                    Some((key, value))
                })
                .collect::<HashMap<_, _>>();
            let rate = pinned
                .unit_price(
                    window.metric.id,
                    window.sub_component_id,
                    window.sub_add_on_id,
                    &dimensions,
                )
                .unwrap_or(*rate);
            rate * quantity
        }
        UsagePricingModel::Matrix { rates } => rates
            .iter()
            .find(|rate| matrix_cell_matches(rate, &event.properties))
            .map_or(Decimal::ZERO, |rate| rate.per_unit_price * quantity),
        _ => return None,
    };
    Some(cost_in(window, amount))
}

/// `amount` in what `window` bills: its credit unit when credit-priced, else its currency.
fn cost_in(window: &PlannedWindow, amount: Decimal) -> UsageCost {
    UsageCost {
        amount,
        currency: window.currency.clone(),
        unit: if window.overage.is_some() {
            CostUnit::CreditUnit
        } else {
            CostUnit::Currency
        },
    }
}

fn rating_key(dimensions: &HashMap<String, String>) -> String {
    let mut entries = dimensions.iter().collect::<Vec<_>>();
    entries.sort();
    entries
        .into_iter()
        .map(|(key, value)| format!("{key}\u{1f}{value}"))
        .collect::<Vec<_>>()
        .join("\u{1e}")
}

/// Splits `total` minor units by `weights`, largest remainder first, so the parts add up to it.
fn allocate(total: i64, weights: &[Decimal]) -> Vec<i64> {
    let sum: Decimal = weights.iter().sum();
    if weights.len() <= 1 || sum <= Decimal::ZERO || weights.iter().any(|w| *w < Decimal::ZERO) {
        let mut parts = vec![0; weights.len()];
        if let Some(first) = parts.first_mut() {
            *first = total;
        }
        return parts;
    }
    let exact = weights
        .iter()
        .map(|weight| Decimal::from(total) * weight / sum)
        .collect::<Vec<_>>();
    let mut parts = exact
        .iter()
        .map(|share| {
            use rust_decimal::prelude::ToPrimitive;
            share.floor().to_i64().unwrap_or(0)
        })
        .collect::<Vec<_>>();
    let mut leftover = total - parts.iter().sum::<i64>();
    let mut order = (0..weights.len()).collect::<Vec<_>>();
    order.sort_by_key(|index| Reverse(exact[*index] - exact[*index].floor()));
    for index in order {
        if leftover <= 0 {
            break;
        }
        parts[index] += 1;
        leftover -= 1;
    }
    parts
}

#[derive(Debug, Clone)]
struct BucketPoint {
    bucket_start: NaiveDateTime,
    until: NaiveDateTime,
    value: Decimal,
    dimensions: HashMap<String, String>,
}

struct Cumulative {
    dimensions: HashMap<String, String>,
    units: Decimal,
    minor: i64,
}

#[derive(Default)]
struct RowAcc {
    quantity: Decimal,
    cost: Option<UsageCost>,
    exact: bool,
}

struct Scope {
    customer_id: CustomerId,
    metric: BillableMetric,
    windows: Vec<PlannedWindow>,
    pins: HashMap<(SubscriptionId, NaiveDate), PinnedPrices>,
}

impl Scope {
    fn pinned_for(&self, window: &PlannedWindow) -> PinnedPrices {
        self.pins
            .get(&(window.subscription_id, window.period.start))
            .cloned()
            .unwrap_or_default()
    }
}

/// A run of one metric's windows read as a single usage query.
struct Span<'a> {
    window: &'a PlannedWindow,
    start: NaiveDateTime,
    end: NaiveDateTime,
}

/// The reads the windows call for within the range. A metric that cannot be summed is not
/// priced, and a bucket cannot be assembled from the parts of two reads, so consecutive
/// windows of it are read as one.
fn usage_spans(
    windows: &[PlannedWindow],
    range_start: NaiveDateTime,
    range_end: NaiveDateTime,
) -> Vec<Span<'_>> {
    let mut spans: Vec<Span> = Vec::new();
    for window in windows {
        let start = window.start.max(range_start);
        let end = window.end.min(range_end);
        if start >= end {
            continue;
        }
        match spans.last_mut() {
            Some(last)
                if !foldable(&window.metric.aggregation_type)
                    && last.window.metric.id == window.metric.id
                    && last.window.metric.usage_scope == window.metric.usage_scope
                    && start <= last.end =>
            {
                last.end = last.end.max(end);
            }
            _ => spans.push(Span { window, start, end }),
        }
    }
    spans
}

impl ServicesEdge {
    pub async fn list_metric_events_with_cost(
        &self,
        tenant_id: TenantId,
        owner: UsageOwner,
        metric: MetricRef,
        from: NaiveDateTime,
        to: NaiveDateTime,
        after: Option<MetricEventCursor>,
        limit: u32,
    ) -> StoreResult<MetricEventsPage> {
        validate_range(from, to)?;
        let scope = self
            .metric_scope(tenant_id, owner, metric, from, to)
            .await?;
        // A subscription lists the events its billing periods price, never the customer's others.
        let (from, to) = match owner {
            UsageOwner::Customer(_) => (from, to),
            UsageOwner::Subscription(_) => {
                let start = scope.windows.iter().map(|window| window.start).min();
                let end = scope.windows.iter().map(|window| window.end).max();
                let (start, end) = start.zip(end).ok_or_else(|| {
                    invalid("the subscription has no billing period pricing this metric in the requested range")
                })?;
                (from.max(start), to.min(end))
            }
        };

        let limit = limit.clamp(1, MAX_EVENTS_PAGE);
        let mut events = self
            .services
            .usage_client
            .list_metric_events(
                &tenant_id,
                &scope.customer_id,
                &scope.metric,
                UsagePeriod {
                    start: from,
                    end: to,
                },
                after,
                limit + 1,
            )
            .await?;

        let has_more = events.len() > limit as usize;
        events.truncate(limit as usize);
        let next = match (has_more, events.last()) {
            (true, Some(last)) => Some(MetricEventCursor {
                timestamp_nanos: last
                    .timestamp
                    .and_utc()
                    .timestamp_nanos_opt()
                    .ok_or_else(|| invalid("event timestamp out of range"))?,
                id: last.id.clone(),
            }),
            _ => None,
        };

        let events = events
            .into_iter()
            .map(|event| {
                let window = scope.windows.iter().find(|window| {
                    window.start <= event.timestamp
                        && event.timestamp < window.end
                        && window
                            .metric
                            .usage_scope
                            .contains(&event.properties, event.timestamp)
                });
                let metric = window.map_or(&scope.metric, |window| &window.metric);
                let quantity = event_units(metric, &event).map(|units| billed_units(metric, units));
                let cost = window.zip(quantity).and_then(|(window, quantity)| {
                    event_cost(window, &scope.pinned_for(window), &event, quantity)
                });
                MetricEventCost {
                    event,
                    quantity,
                    cost,
                }
            })
            .collect();

        Ok(MetricEventsPage { events, next })
    }

    pub async fn usage_costs(
        &self,
        tenant_id: TenantId,
        owner: UsageOwner,
        metric: MetricRef,
        group_by: Option<String>,
        granularity: CostGranularity,
        from: NaiveDateTime,
        to: NaiveDateTime,
    ) -> StoreResult<Vec<UsageCostRow>> {
        validate_range(from, to)?;
        if group_by
            .as_deref()
            .is_some_and(|key| key.is_empty() || key.len() > MAX_GROUP_BY_LEN)
        {
            return Err(invalid("`group_by` must be a property name"));
        }
        let range_start = bucket_floor(from, granularity);
        let range_end = bucket_ceil(to, granularity);

        let scope = self
            .metric_scope(tenant_id, owner, metric, range_start, range_end)
            .await?;
        if scope.windows.is_empty() {
            let owner = match owner {
                UsageOwner::Customer(_) => "customer",
                UsageOwner::Subscription(_) => "subscription",
            };
            return Err(invalid(format!(
                "the {owner} has no billing period pricing this metric in the requested range"
            )));
        }
        let customer_id = scope.customer_id;

        let mut rows: BTreeMap<(NaiveDateTime, Option<String>), RowAcc> = BTreeMap::new();
        for span in usage_spans(&scope.windows, range_start, range_end) {
            let points = self
                .bucket_points(
                    tenant_id,
                    customer_id,
                    &span.window.metric,
                    group_by.as_deref(),
                    granularity,
                    span.start,
                    span.end,
                )
                .await?;
            self.rate_points(
                tenant_id,
                customer_id,
                span.window,
                &scope.pinned_for(span.window),
                group_by.as_deref(),
                span.start,
                points,
                &mut rows,
            )
            .await?;
        }

        rows.into_iter()
            .map(|((bucket_start, group), acc)| {
                Ok(UsageCostRow {
                    bucket_start,
                    group,
                    quantity: acc.quantity,
                    cost: acc.cost,
                    exact: acc.exact,
                })
            })
            .collect()
    }

    async fn resolve_metric(
        &self,
        tenant_id: TenantId,
        metric: MetricRef,
    ) -> StoreResult<BillableMetric> {
        match metric {
            MetricRef::Id(id) => self.store.find_billable_metric_by_id(id, tenant_id).await,
            MetricRef::Code(code) => {
                let mut metrics = self
                    .store
                    .list_billable_metrics_by_code(tenant_id, code.clone())
                    .await?;
                metrics.sort_by_key(|metric| {
                    (metric.archived_at.is_some(), Reverse(metric.created_at))
                });
                metrics.into_iter().next().ok_or_else(|| {
                    Report::new(StoreError::ValueNotFound(format!(
                        "billable metric with code {code}"
                    )))
                })
            }
        }
    }

    async fn metric_scope(
        &self,
        tenant_id: TenantId,
        owner: UsageOwner,
        metric: MetricRef,
        from: NaiveDateTime,
        to: NaiveDateTime,
    ) -> StoreResult<Scope> {
        let by_id = matches!(metric, MetricRef::Id(_));
        let mut metric = self.resolve_metric(tenant_id, metric).await?;
        let same_metric = |metric: &BillableMetric, other: &BillableMetric| {
            if by_id {
                other.id == metric.id
            } else {
                other.code == metric.code
            }
        };
        let mut conn = self.store.get_conn().await?;
        let (customer_id, subscription) = match owner {
            UsageOwner::Customer(customer_id) => (customer_id, None),
            UsageOwner::Subscription(subscription_id) => {
                let details = self
                    .store
                    .get_subscription_details_with_conn(&mut conn, tenant_id, subscription_id)
                    .await?;
                (details.subscription.customer_id, Some(details))
            }
        };
        let subscription_id = subscription.as_ref().map(|details| details.subscription.id);
        let windows = self
            .customer_windows(&mut conn, tenant_id, customer_id, from.date(), to.date())
            .await?
            .into_iter()
            .filter(|window| subscription_id.is_none_or(|id| window.subscription_id == id))
            .collect::<Vec<_>>();
        if let Some(details) = subscription {
            // Scoped by the subscription's usage filter, so its reads count its events only; a
            // metric a plan change removed is still billed by the windows that closed it.
            metric = details
                .metrics
                .iter()
                .chain(windows.iter().map(|window| &window.metric))
                .find(|other| same_metric(&metric, other))
                .cloned()
                .ok_or_else(|| invalid("the subscription does not bill this metric"))?;
        }
        let mut windows = windows
            .into_iter()
            .filter(|window| {
                same_metric(&metric, &window.metric) && window.start < to && window.end > from
            })
            .collect::<Vec<_>>();
        windows.sort_by_key(|window| window.start);
        // Subscriptions told apart by a usage filter price disjoint events at the same time.
        let clash = |a: &PlannedWindow, b: &PlannedWindow| {
            a.start < b.end
                && b.start < a.end
                && (a.subscription_id == b.subscription_id
                    || (!a.metric.usage_scope.is_filtered() && !b.metric.usage_scope.is_filtered()))
        };
        if windows
            .iter()
            .enumerate()
            .any(|(index, a)| windows[index + 1..].iter().any(|b| clash(a, b)))
        {
            return Err(invalid(
                "several fees price this metric at the same time for the customer",
            ));
        }
        let pins = if windows.is_empty() {
            HashMap::new()
        } else {
            draft_pinned_prices(&mut conn, tenant_id, customer_id).await?
        };
        Ok(Scope {
            customer_id,
            metric,
            windows,
            pins,
        })
    }

    /// What each metric's usage adds so far to the subscription's next invoice, priced as the
    /// invoicer prices it: usage fees only, before discounts and credits.
    pub async fn subscription_running_total(
        &self,
        details: &SubscriptionDetails,
    ) -> StoreResult<RunningTotal> {
        let now = Utc::now().naive_utc();
        let computed_at = now.with_nanosecond(0).unwrap_or(now);
        let tenant_id = details.subscription.tenant_id;
        let customer_id = details.subscription.customer_id;
        let (windows, pins) = {
            let mut conn = self.store.get_conn().await?;
            let windows = self.next_invoice_windows(&mut conn, details).await?;
            let pins = if windows.is_empty() {
                HashMap::new()
            } else {
                draft_pinned_prices(&mut conn, tenant_id, customer_id).await?
            };
            (windows, pins)
        };

        let usage_client = &self.services.usage_client;
        let pins = &pins;
        let reads = windows
            .iter()
            .filter(|window| window.end.min(computed_at) > window.start)
            .map(|window| async move {
                let until = window.end.min(computed_at);
                let usage = usage_client
                    .fetch_usage(
                        &tenant_id,
                        &customer_id,
                        &window.metric,
                        UsagePeriod {
                            start: window.start,
                            end: until,
                        },
                    )
                    .await?;
                let pinned = pins
                    .get(&(window.subscription_id, window.period.start))
                    .cloned()
                    .unwrap_or_default();
                let minor = price_window_groups(window, until, usage.data, &pinned)?;
                Ok::<_, Report<StoreError>>((
                    window,
                    Decimal::new(minor, u32::from(window.precision)),
                ))
            })
            .collect::<Vec<_>>();
        let priced: Vec<(&PlannedWindow, Decimal)> = stream::iter(reads)
            .buffered(USAGE_FETCH_CONCURRENCY)
            .try_collect()
            .await?;

        let mut metrics: Vec<MetricRunningCost> = Vec::new();
        for (window, amount) in priced {
            let index = match metrics.iter().position(|m| m.metric.id == window.metric.id) {
                Some(index) => index,
                None => {
                    metrics.push(MetricRunningCost {
                        metric: window.metric.clone(),
                        costs: Vec::new(),
                    });
                    metrics.len() - 1
                }
            };
            let cost = cost_in(window, amount);
            let costs = &mut metrics[index].costs;
            match costs
                .iter_mut()
                .find(|known| known.currency == cost.currency)
            {
                Some(known) => known.amount += cost.amount,
                None => costs.push(cost),
            }
        }
        Ok(RunningTotal {
            computed_at,
            metrics,
        })
    }

    /// The usage windows of the subscription's next invoice, on the basis its preview is computed
    /// on: none while it invoices nothing, the final invoice's when a stop is scheduled first.
    async fn next_invoice_windows(
        &self,
        conn: &mut PgConn,
        details: &SubscriptionDetails,
    ) -> StoreResult<Vec<PlannedWindow>> {
        if !matches!(
            details.subscription.status,
            SubscriptionStatusEnum::Active
                | SubscriptionStatusEnum::TrialActive
                | SubscriptionStatusEnum::PendingCharge
        ) {
            return Ok(Vec::new());
        }
        let basis = self.services.next_invoice_basis(conn, details).await?;
        let span = basis.span();
        let (billed, invoice_date, span) = match basis.change {
            Some(
                UpcomingChange::Ends { billed: false, .. }
                | UpcomingChange::Pauses { billed: false, .. },
            ) => return Ok(Vec::new()),
            Some(UpcomingChange::Ends { .. } | UpcomingChange::Pauses { .. }) | None => {
                (basis.details, basis.invoice_date, span)
            }
            // The change applies at the period's end: the usage up to it is billed as it is now.
            Some(_) => {
                let (billed, invoice_date) = upcoming_invoice_details_on(conn, details).await?;
                (billed, invoice_date, BilledSpan::Period)
            }
        };
        let cycle_index = billed.subscription.cycle_index.unwrap_or(0);
        windows_at(conn, &billed, invoice_date, cycle_index, span).await
    }

    /// The usage windows of the customer's upcoming invoices and of its recurring invoices dated
    /// in `[anchor_from, anchor_before + one billing period)`, derived by the invoicer's own
    /// period math.
    async fn customer_windows(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        anchor_from: NaiveDate,
        anchor_before: NaiveDate,
    ) -> StoreResult<Vec<PlannedWindow>> {
        let mut details = HashMap::new();
        let mut windows = Vec::new();

        let active = SubscriptionRow::find_active_by_customer(conn, customer_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        for subscription in active {
            let subscription_details = self
                .store
                .get_subscription_details_with_conn(conn, tenant_id, subscription.id)
                .await?;
            windows.extend(
                self.next_invoice_windows(conn, &subscription_details)
                    .await?,
            );
            details.insert(subscription.id, subscription_details);
        }

        let cycles = list_recurring_invoice_cycles(
            conn,
            tenant_id,
            customer_id,
            anchor_from,
            anchor_before + MAX_BILLING_PERIOD,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        for cycle in cycles {
            // Invoices that do not pin their cycle cannot be replayed without guessing it.
            let Some(cycle_index) = cycle
                .cycle_index
                .and_then(|index| u32::try_from(index).ok())
            else {
                continue;
            };
            if let Entry::Vacant(slot) = details.entry(cycle.subscription_id) {
                slot.insert(
                    self.store
                        .get_subscription_details_with_conn(conn, tenant_id, cycle.subscription_id)
                        .await?,
                );
            }
            let subscription_details = &details[&cycle.subscription_id];
            let invoice_type: crate::domain::enums::InvoiceType = cycle.invoice_type.clone().into();
            windows.extend(
                windows_at(
                    conn,
                    subscription_details,
                    cycle.anchor,
                    cycle_index,
                    BilledSpan::of(&invoice_type),
                )
                .await?,
            );
        }

        let mut seen = HashSet::new();
        windows.retain(|window| {
            seen.insert((
                window.subscription_id,
                window.sub_component_id.map(|id| id.as_uuid()),
                window.sub_add_on_id.map(|id| id.as_uuid()),
                window.metric.id,
                window.period.start,
            ))
        });
        Ok(windows)
    }

    async fn bucket_points(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        metric: &BillableMetric,
        group_by: Option<&str>,
        granularity: CostGranularity,
        start: NaiveDateTime,
        end: NaiveDateTime,
    ) -> StoreResult<Vec<BucketPoint>> {
        let usage_client = &self.services.usage_client;
        let mut points = Vec::new();
        match granularity {
            CostGranularity::Hour | CostGranularity::Day => {
                let grain = if granularity == CostGranularity::Hour {
                    TenantUsageGrain::Hour
                } else {
                    TenantUsageGrain::Day
                };
                let series = usage_client
                    .fetch_usage_series(
                        &tenant_id,
                        &customer_id,
                        metric,
                        group_by,
                        grain,
                        UsagePeriod { start, end },
                    )
                    .await?;
                points.extend(series.into_iter().map(|point| BucketPoint {
                    bucket_start: point.window_start,
                    until: bucket_end(point.window_start, granularity).min(end),
                    value: point.value,
                    dimensions: point.dimensions,
                }));
            }
            CostGranularity::Month => {
                let mut bucket = bucket_floor(start, granularity);
                while bucket < end {
                    let segment = UsagePeriod {
                        start: bucket.max(start),
                        end: bucket_end(bucket, granularity).min(end),
                    };
                    let usage = match group_by {
                        Some(group_by) => {
                            usage_client
                                .fetch_usage_grouped_by(
                                    &tenant_id,
                                    &customer_id,
                                    metric,
                                    group_by,
                                    segment.clone(),
                                )
                                .await?
                        }
                        None => {
                            usage_client
                                .fetch_usage(&tenant_id, &customer_id, metric, segment.clone())
                                .await?
                        }
                    };
                    points.extend(usage.data.into_iter().map(|group| BucketPoint {
                        bucket_start: bucket,
                        until: segment.end,
                        value: group.value,
                        dimensions: group.dimensions,
                    }));
                    bucket = bucket_end(bucket, granularity);
                }
            }
        }
        points.sort_by_key(|point| point.bucket_start);
        Ok(points)
    }

    #[allow(clippy::too_many_arguments)]
    async fn rate_points(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        window: &PlannedWindow,
        pinned: &PinnedPrices,
        group_by: Option<&str>,
        start: NaiveDateTime,
        points: Vec<BucketPoint>,
        rows: &mut BTreeMap<(NaiveDateTime, Option<String>), RowAcc>,
    ) -> StoreResult<()> {
        let metric = &window.metric;
        let group_of =
            |point: &BucketPoint| group_by.and_then(|key| point.dimensions.get(key).cloned());

        if !foldable(&metric.aggregation_type) {
            for point in points {
                let quantity = billed_units(metric, point.value);
                let key = (point.bucket_start, group_of(&point));
                if rows.contains_key(&key) {
                    return Err(invalid(
                        "this metric's aggregation cannot be combined across groups; group by the metric's grouping property",
                    ));
                }
                rows.entry(key).or_default().merge(quantity, None, false)?;
            }
            return Ok(());
        }

        let linear = matches!(
            &window.fee,
            MeteredFee::Usage(UsagePricingModel::PerUnit { .. } | UsagePricingModel::Matrix { .. })
        ) && !rounds_converted_units(metric);
        let price = |dimensions: &HashMap<String, String>, units: Decimal, until| {
            if units <= Decimal::ZERO {
                return Ok(0);
            }
            price_window_groups(
                window,
                until,
                vec![GroupedUsageData {
                    value: units,
                    dimensions: dimensions.clone(),
                }],
                pinned,
            )
        };

        let extra_key = group_by.filter(|key| {
            metric.usage_group_key.as_deref() != Some(*key)
                && !segmentation_keys(metric).contains(key)
        });
        let rating_dimensions = |point: &BucketPoint| {
            let mut dimensions = point.dimensions.clone();
            if let Some(key) = extra_key {
                dimensions.remove(key);
            }
            dimensions
        };

        let mut cumulative: HashMap<String, Cumulative> = HashMap::new();
        if start > window.start {
            let prior = self
                .services
                .usage_client
                .fetch_usage(
                    &tenant_id,
                    &customer_id,
                    metric,
                    UsagePeriod {
                        start: window.start,
                        end: start,
                    },
                )
                .await?;
            for group in prior.data {
                let entry = cumulative
                    .entry(rating_key(&group.dimensions))
                    .or_insert_with(|| Cumulative {
                        dimensions: group.dimensions.clone(),
                        units: Decimal::ZERO,
                        minor: 0,
                    });
                entry.units += group.value;
            }
            for entry in cumulative.values_mut() {
                entry.minor = price(&entry.dimensions, entry.units, start)?;
            }
        }

        let mut buckets: BTreeMap<NaiveDateTime, Vec<&BucketPoint>> = BTreeMap::new();
        for point in &points {
            buckets.entry(point.bucket_start).or_default().push(point);
        }
        for (bucket_start, bucket_points) in buckets {
            let mut rating_groups: BTreeMap<String, Vec<&BucketPoint>> = BTreeMap::new();
            for point in bucket_points {
                rating_groups
                    .entry(rating_key(&rating_dimensions(point)))
                    .or_default()
                    .push(point);
            }
            for (key, group_points) in rating_groups {
                let until = group_points
                    .iter()
                    .map(|point| point.until)
                    .max()
                    .unwrap_or(bucket_start);
                let dimensions = rating_dimensions(group_points[0]);
                let entry = cumulative.entry(key).or_insert_with(|| Cumulative {
                    dimensions: dimensions.clone(),
                    units: Decimal::ZERO,
                    minor: 0,
                });
                entry.units += group_points
                    .iter()
                    .map(|point| point.value)
                    .sum::<Decimal>();
                let after = price(&entry.dimensions, entry.units, until)?;
                let delta = after
                    .checked_sub(entry.minor)
                    .ok_or_else(|| invalid("usage cost exceeds the supported range"))?;
                entry.minor = after;

                let quantities = group_points
                    .iter()
                    .map(|point| billed_units(metric, point.value))
                    .collect::<Vec<_>>();
                let shares = allocate(delta, &quantities);
                for ((point, quantity), share) in group_points.iter().zip(quantities).zip(shares) {
                    let amount = Decimal::new(share, u32::from(window.precision));
                    let acc = rows.entry((bucket_start, group_of(point))).or_default();
                    acc.merge(quantity, Some(cost_in(window, amount)), linear)?;
                }
            }
        }
        Ok(())
    }
}

/// The unit prices operators pinned on the customer's draft invoices, by the period they bill.
async fn draft_pinned_prices(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> StoreResult<HashMap<(SubscriptionId, NaiveDate), PinnedPrices>> {
    let mut pins: HashMap<(SubscriptionId, NaiveDate), PinnedPrices> = HashMap::new();
    // A consolidated draft's lines are those its parent billed, the parent naming no subscription.
    for draft in
        list_draft_recurring_periods(conn, tenant_id, customer_id, ConsolidatedDrafts::Include)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
    {
        let lines = serde_json::from_value::<Vec<LineItem>>(draft.line_items).map_err(|error| {
            Report::new(StoreError::SerdeError(
                "Failed to deserialize line_items".to_owned(),
                error,
            ))
        })?;
        let mut by_period: HashMap<NaiveDate, Vec<LineItem>> = HashMap::new();
        for line in lines.into_iter().filter(is_usage_based_line) {
            by_period.entry(line.start_date).or_default().push(line);
        }
        for (period_start, lines) in by_period {
            pins.entry((draft.subscription_id, period_start))
                .or_default()
                .add_lines(&lines, draft.tax_inclusive);
        }
    }
    Ok(pins)
}

impl RowAcc {
    fn merge(
        &mut self,
        quantity: Decimal,
        cost: Option<UsageCost>,
        exact: bool,
    ) -> StoreResult<()> {
        let first = self.quantity.is_zero() && self.cost.is_none();
        self.quantity += quantity;
        match (&mut self.cost, cost) {
            (Some(known), Some(cost)) => {
                if known.currency != cost.currency {
                    return Err(invalid("the customer is billed in several currencies"));
                }
                known.amount += cost.amount;
            }
            (known, cost) => {
                if known.is_none() {
                    *known = cost;
                }
            }
        }
        self.exact = if first { exact } else { self.exact && exact };
        Ok(())
    }
}

/// One UTC day of a window: the usage up to `until` (the next midnight, or the window's end) is
/// what the day closes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DayCut {
    date: NaiveDate,
    until: NaiveDateTime,
}

/// The UTC days `[from, to)` spans.
fn day_cuts(from: NaiveDateTime, to: NaiveDateTime) -> Vec<DayCut> {
    let mut cuts = Vec::new();
    let mut date = from.date();
    while date.and_time(NaiveTime::MIN) < to && cuts.len() < MAX_WINDOW_DAYS {
        let next = date + Duration::days(1);
        cuts.push(DayCut {
            date,
            until: next.and_time(NaiveTime::MIN).min(to),
        });
        date = next;
    }
    cuts
}

/// The usage a window reads as of a day's end, from its start.
#[derive(Debug, Clone)]
struct Closing {
    cut: DayCut,
    usage: Vec<GroupedUsageData>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DailyCost {
    date: NaiveDate,
    quantity: Decimal,
    amount_minor: i64,
}

fn billed_total(metric: &BillableMetric, usage: &[GroupedUsageData]) -> Decimal {
    usage
        .iter()
        .map(|group| billed_units(metric, group.value))
        .sum()
}

/// What each day adds to the window: the price of its usage up to the day's end less the price up
/// to the day's start, both priced as the invoice prices the window. The days add up to the
/// window's price and units whatever the pricing model (tiers, volume, packages, capacity); a day
/// is negative when a volume tier lowers the rate of the whole period. `opening` is the usage from
/// the window's start up to the first day, when the days start later.
fn cumulative_daily_costs(
    window: &PlannedWindow,
    pinned: &PinnedPrices,
    opening: Option<(NaiveDateTime, Vec<GroupedUsageData>)>,
    closings: Vec<Closing>,
) -> StoreResult<Vec<DailyCost>> {
    let (mut cost, mut units) = match opening {
        Some((until, usage)) => {
            let units = billed_total(&window.metric, &usage);
            (price_window_groups(window, until, usage, pinned)?, units)
        }
        None => (0, Decimal::ZERO),
    };
    let mut days = Vec::with_capacity(closings.len());
    for closing in closings {
        let closing_units = billed_total(&window.metric, &closing.usage);
        let closing_cost = price_window_groups(window, closing.cut.until, closing.usage, pinned)?;
        days.push(DailyCost {
            date: closing.cut.date,
            quantity: closing_units - units,
            amount_minor: closing_cost
                .checked_sub(cost)
                .ok_or_else(|| invalid("usage cost exceeds the supported range"))?,
        });
        cost = closing_cost;
        units = closing_units;
    }
    Ok(days)
}

fn add_usage(
    running: &mut BTreeMap<String, GroupedUsageData>,
    value: Decimal,
    dimensions: &HashMap<String, String>,
) {
    running
        .entry(rating_key(dimensions))
        .or_insert_with(|| GroupedUsageData {
            value: Decimal::ZERO,
            dimensions: dimensions.clone(),
        })
        .value += value;
}

/// A summable metric's usage at each day's end: the opening plus the daily points so far, per
/// group of the dimensions it is priced by.
fn running_usage(
    opening: &[GroupedUsageData],
    points: &[UsageSeriesPoint],
    cuts: &[DayCut],
) -> Vec<Closing> {
    let mut running = BTreeMap::new();
    for group in opening {
        add_usage(&mut running, group.value, &group.dimensions);
    }
    let mut by_day: BTreeMap<NaiveDate, Vec<&UsageSeriesPoint>> = BTreeMap::new();
    for point in points {
        by_day
            .entry(point.window_start.date())
            .or_default()
            .push(point);
    }
    cuts.iter()
        .map(|cut| {
            for point in by_day.remove(&cut.date).unwrap_or_default() {
                add_usage(&mut running, point.value, &point.dimensions);
            }
            Closing {
                cut: *cut,
                usage: running.values().cloned().collect(),
            }
        })
        .collect()
}

/// A window with the instants its usage is read over.
struct ReadWindow {
    window: PlannedWindow,
    from: NaiveDateTime,
    to: NaiveDateTime,
}

impl ServicesEdge {
    /// The wallet's usage per day, subscription, metric and product over `span`: every usage
    /// window charged in the wallet's denomination (its currency, or its credit unit for usage
    /// priced in credits), free fees aside, priced as its invoice prices it, before tax, and
    /// attributed to days by `cumulative_daily_costs`. With `subscription_id`, that subscription's
    /// windows only.
    pub async fn wallet_daily_usage_costs(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        wallet_id: WalletId,
        subscription_id: Option<SubscriptionId>,
        span: WalletUsageSpan,
    ) -> StoreResult<WalletDailyUsage> {
        let now = Utc::now().naive_utc();
        let today_ends = (now.date() + Duration::days(1)).and_time(NaiveTime::MIN);
        let (windows, pins) = {
            let mut conn = self.store.get_conn().await?;
            let wallet = CustomerCreditAccountRow::find_by_id(&mut conn, tenant_id, wallet_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .filter(|account| account.customer_id == customer_id)
                .ok_or_else(|| Report::new(StoreError::ValueNotFound("wallet".to_owned())))?;
            if let Some(subscription_id) = subscription_id {
                let subscription =
                    SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .subscription;
                if subscription.customer_id != customer_id {
                    return Err(invalid("the subscription belongs to another customer"));
                }
            }
            let denomination = normalized_currency(&wallet.currency);
            let windows = self
                .span_windows(&mut conn, tenant_id, customer_id, subscription_id, span)
                .await?
                .into_iter()
                .filter(|read| read.window.currency == denomination && !read.window.fee.is_free())
                .collect::<Vec<_>>();
            let pins = if windows.is_empty() {
                HashMap::new()
            } else {
                draft_pinned_prices(&mut conn, tenant_id, customer_id).await?
            };
            (windows, pins)
        };

        let (from, to) = match span {
            WalletUsageSpan::Days { from, to } => (Some(from), Some(to)),
            WalletUsageSpan::CurrentPeriod | WalletUsageSpan::PreviousPeriod => (
                windows.iter().map(|read| read.from.date()).min(),
                windows.iter().map(|read| day_after(read.to)).max(),
            ),
        };

        let mut totals: BTreeMap<
            (NaiveDate, uuid::Uuid, uuid::Uuid, Option<uuid::Uuid>),
            WalletDailyUsageCost,
        > = BTreeMap::new();
        for read in &windows {
            let window = &read.window;
            let pinned = pins
                .get(&(window.subscription_id, window.period.start))
                .cloned()
                .unwrap_or_default();
            let days = self
                .window_daily_costs(
                    tenant_id,
                    customer_id,
                    window,
                    &pinned,
                    read.from,
                    read.to.min(today_ends),
                )
                .await?;
            for day in days {
                if day.amount_minor == 0 && day.quantity.is_zero() {
                    continue;
                }
                let row = totals
                    .entry((
                        day.date,
                        window.subscription_id.as_uuid(),
                        window.metric.id.as_uuid(),
                        window.product_id.map(|id| id.as_uuid()),
                    ))
                    .or_insert_with(|| WalletDailyUsageCost {
                        date: day.date,
                        subscription_id: window.subscription_id,
                        metric_id: window.metric.id,
                        metric_name: window.metric.name.clone(),
                        product_id: window.product_id,
                        quantity: Decimal::ZERO,
                        amount_minor: 0,
                    });
                row.quantity += day.quantity;
                row.amount_minor = row
                    .amount_minor
                    .checked_add(day.amount_minor)
                    .ok_or_else(|| invalid("usage cost exceeds the supported range"))?;
            }
        }
        Ok(WalletDailyUsage {
            from,
            to,
            rows: totals.into_values().collect(),
        })
    }

    /// The windows `span` covers for the customer's subscriptions (or `subscription_id`'s), each
    /// with the instants it is read over.
    async fn span_windows(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        subscription_id: Option<SubscriptionId>,
        span: WalletUsageSpan,
    ) -> StoreResult<Vec<ReadWindow>> {
        let in_scope = |id: SubscriptionId| subscription_id.is_none_or(|scoped| scoped == id);
        let whole = |window: PlannedWindow| ReadWindow {
            from: window.start,
            to: window.end,
            window,
        };
        match span {
            WalletUsageSpan::CurrentPeriod => {
                let active = SubscriptionRow::find_active_by_customer(conn, customer_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let mut windows = Vec::new();
                for subscription in active.iter().filter(|s| in_scope(s.id)) {
                    let details = self
                        .store
                        .get_subscription_details_with_conn(conn, tenant_id, subscription.id)
                        .await?;
                    windows.extend(self.next_invoice_windows(conn, &details).await?);
                }
                Ok(windows.into_iter().map(whole).collect())
            }
            WalletUsageSpan::PreviousPeriod => {
                let active = SubscriptionRow::find_active_by_customer(conn, customer_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .into_iter()
                    .filter(|s| in_scope(s.id))
                    .collect::<Vec<_>>();
                let earliest = active.iter().map(|s| s.current_period_start).min();
                let latest = active.iter().map(|s| s.current_period_start).max();
                let (Some(earliest), Some(latest)) = (earliest, latest) else {
                    return Ok(Vec::new());
                };
                let cycles = list_recurring_invoice_cycles(
                    conn,
                    tenant_id,
                    customer_id,
                    earliest,
                    latest + Duration::days(1),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                let mut windows = Vec::new();
                for subscription in &active {
                    let Some(cycle_index) =
                        opening_cycle(&cycles, subscription.id, subscription.current_period_start)
                    else {
                        continue;
                    };
                    let details = self
                        .store
                        .get_subscription_details_with_conn(conn, tenant_id, subscription.id)
                        .await?;
                    windows.extend(
                        windows_at(
                            conn,
                            &details,
                            subscription.current_period_start,
                            cycle_index,
                            BilledSpan::Period,
                        )
                        .await?,
                    );
                }
                Ok(windows.into_iter().map(whole).collect())
            }
            WalletUsageSpan::Days { from, to } => {
                let start = from.and_time(NaiveTime::MIN);
                let end = to.and_time(NaiveTime::MIN);
                validate_range(start, end)?;
                Ok(self
                    .customer_windows(conn, tenant_id, customer_id, from, to)
                    .await?
                    .into_iter()
                    .filter(|window| {
                        in_scope(window.subscription_id) && window.start < end && window.end > start
                    })
                    .map(|window| ReadWindow {
                        from: window.start.max(start),
                        to: window.end.min(end),
                        window,
                    })
                    .collect())
            }
        }
    }

    /// What each day of `[from, to)` adds to the window's cost (see `cumulative_daily_costs`). A
    /// summable metric is read once per day; another is read from the window's start up to the
    /// end of each day it received events, those reads running side by side.
    async fn window_daily_costs(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        window: &PlannedWindow,
        pinned: &PinnedPrices,
        from: NaiveDateTime,
        to: NaiveDateTime,
    ) -> StoreResult<Vec<DailyCost>> {
        let cuts = day_cuts(from, to);
        let Some(last) = cuts.last() else {
            return Ok(Vec::new());
        };
        let usage_client = &self.services.usage_client;
        let metric = &window.metric;
        let opening = if from > window.start {
            Some(
                usage_client
                    .fetch_usage(
                        &tenant_id,
                        &customer_id,
                        metric,
                        UsagePeriod {
                            start: window.start,
                            end: from,
                        },
                    )
                    .await?
                    .data,
            )
        } else {
            None
        };
        let series = usage_client
            .fetch_usage_series(
                &tenant_id,
                &customer_id,
                metric,
                None,
                TenantUsageGrain::Day,
                UsagePeriod {
                    start: from,
                    end: last.until,
                },
            )
            .await?;
        let closings = if foldable(&metric.aggregation_type) {
            running_usage(opening.as_deref().unwrap_or_default(), &series, &cuts)
        } else {
            let active = series
                .iter()
                .map(|point| point.window_start.date())
                .collect::<HashSet<_>>();
            let reads = cuts
                .iter()
                .filter(|cut| active.contains(&cut.date))
                .map(|cut| {
                    usage_client.fetch_usage(
                        &tenant_id,
                        &customer_id,
                        metric,
                        UsagePeriod {
                            start: window.start,
                            end: cut.until,
                        },
                    )
                })
                .collect::<Vec<_>>();
            let mut read = stream::iter(reads)
                .buffered(USAGE_FETCH_CONCURRENCY)
                .try_collect::<Vec<_>>()
                .await?
                .into_iter();
            let mut usage = opening.clone().unwrap_or_default();
            let mut closings = Vec::with_capacity(cuts.len());
            for cut in &cuts {
                if active.contains(&cut.date)
                    && let Some(closing) = read.next()
                {
                    usage = closing.data;
                }
                closings.push(Closing {
                    cut: *cut,
                    usage: usage.clone(),
                });
            }
            closings
        };
        cumulative_daily_costs(window, pinned, opening.map(|usage| (from, usage)), closings)
    }
}

/// The day after the last one `[.., to)` touches.
fn day_after(to: NaiveDateTime) -> NaiveDate {
    if to.time() == NaiveTime::MIN {
        to.date()
    } else {
        to.date() + Duration::days(1)
    }
}

/// The cycle of the subscription's recurring invoice dated at its current period's start: the one
/// billing its previous period's usage, closed fees included.
fn opening_cycle(
    cycles: &[RecurringInvoiceCycle],
    subscription_id: SubscriptionId,
    current_period_start: NaiveDate,
) -> Option<u32> {
    cycles
        .iter()
        .filter(|cycle| {
            cycle.subscription_id == subscription_id
                && cycle.anchor == current_period_start
                && matches!(
                    cycle.invoice_type,
                    diesel_models::enums::InvoiceType::Recurring
                )
        })
        .find_map(|cycle| {
            cycle
                .cycle_index
                .and_then(|index| u32::try_from(index).ok())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Period;
    use crate::domain::enums::BillingMetricAggregateEnum;
    use crate::domain::price_components::TierRow;
    use common_domain::ids::{BillableMetricId, ProductFamilyId};
    use rust_decimal_macros::dec;

    fn june(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 6, 1).unwrap() + Duration::days(i64::from(day) - 1)
    }

    fn midnight(day: u32) -> NaiveDateTime {
        june(day).and_time(NaiveTime::MIN)
    }

    fn metric() -> BillableMetric {
        BillableMetric {
            id: BillableMetricId::new(),
            name: "API calls".to_owned(),
            description: None,
            code: "api_calls".to_owned(),
            aggregation_type: BillingMetricAggregateEnum::Sum,
            aggregation_key: Some("n".to_owned()),
            unit_conversion_factor: None,
            unit_conversion_rounding: None,
            segmentation_matrix: None,
            filters: None,
            usage_group_key: None,
            unit_singular: None,
            unit_plural: None,
            created_at: NaiveDateTime::default(),
            updated_at: None,
            archived_at: None,
            tenant_id: TenantId::new(),
            product_family_id: ProductFamilyId::new(),
            product_id: None,
            usage_scope: Default::default(),
        }
    }

    fn window_over(
        subscription_id: SubscriptionId,
        model: UsagePricingModel,
        start: NaiveDate,
        end: NaiveDate,
    ) -> PlannedWindow {
        PlannedWindow {
            subscription_id,
            metric: metric(),
            fee: MeteredFee::Usage(model),
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: None,
            tax_inclusive: false,
            taxes: Vec::new(),
            period: Period { start, end },
            start: start.and_time(NaiveTime::MIN),
            end: end.and_time(NaiveTime::MIN),
            currency: "USD".to_owned(),
            precision: 2,
            overage: None,
        }
    }

    fn june_window(model: UsagePricingModel) -> PlannedWindow {
        window_over(SubscriptionId::new(), model, june(1), june(31))
    }

    /// 1.00 per unit up to 100 units, 0.50 past them.
    fn two_tiers() -> Vec<TierRow> {
        [(0, dec!(1)), (100, dec!(0.5))]
            .into_iter()
            .map(|(first_unit, rate)| TierRow {
                first_unit,
                rate,
                flat_fee: None,
                flat_cap: None,
            })
            .collect()
    }

    fn usage(value: Decimal) -> Vec<GroupedUsageData> {
        vec![GroupedUsageData {
            value,
            dimensions: HashMap::new(),
        }]
    }

    fn point(day: u32, value: Decimal, region: Option<&str>) -> UsageSeriesPoint {
        UsageSeriesPoint {
            window_start: midnight(day),
            value,
            dimensions: region
                .map(|region| HashMap::from([("region".to_owned(), region.to_owned())]))
                .unwrap_or_default(),
        }
    }

    /// The days of a window read from its start, `units[i]` metered on June `i + 1`.
    fn days_of(window: &PlannedWindow, units: &[Decimal]) -> Vec<DailyCost> {
        let cuts = day_cuts(window.start, midnight(units.len() as u32 + 1));
        let points = units
            .iter()
            .enumerate()
            .map(|(index, value)| point(index as u32 + 1, *value, None))
            .collect::<Vec<_>>();
        cumulative_daily_costs(
            window,
            &PinnedPrices::default(),
            None,
            running_usage(&[], &points, &cuts),
        )
        .unwrap()
    }

    fn period_price(window: &PlannedWindow, units: Decimal) -> i64 {
        price_window_groups(window, window.end, usage(units), &PinnedPrices::default()).unwrap()
    }

    fn amounts(days: &[DailyCost]) -> Vec<i64> {
        days.iter().map(|day| day.amount_minor).collect()
    }

    #[test]
    fn graduated_tiers_give_each_day_what_it_adds_to_the_period_price() {
        let window = june_window(UsagePricingModel::Tiered {
            tiers: two_tiers(),
            block_size: None,
        });
        let days = days_of(&window, &[dec!(60), dec!(60), dec!(60)]);

        // 60 at 1.00; 40 at 1.00 and 20 at 0.50; 60 at 0.50.
        assert_eq!(amounts(&days), vec![6_000, 5_000, 3_000]);
        assert_eq!(
            days.iter().map(|day| day.date).collect::<Vec<_>>(),
            vec![june(1), june(2), june(3)]
        );
        assert_eq!(period_price(&window, dec!(180)), 14_000);
        assert_eq!(
            days.iter().map(|day| day.amount_minor).sum::<i64>(),
            period_price(&window, dec!(180))
        );
        assert_eq!(
            days.iter().map(|day| day.quantity).sum::<Decimal>(),
            dec!(180)
        );
    }

    #[test]
    fn a_volume_tier_reached_mid_period_lowers_that_day_and_the_days_still_sum_to_the_period() {
        let window = june_window(UsagePricingModel::Volume {
            tiers: two_tiers(),
            block_size: None,
        });
        let days = days_of(&window, &[dec!(60), dec!(50), dec!(70)]);

        // 60 at 1.00 is 60.00; 110 at 0.50 is 55.00; 180 at 0.50 is 90.00.
        assert_eq!(amounts(&days), vec![6_000, -500, 3_500]);
        assert_eq!(
            days.iter().map(|day| day.amount_minor).sum::<i64>(),
            period_price(&window, dec!(180))
        );
    }

    #[test]
    fn per_unit_days_cost_their_own_usage() {
        let window = june_window(UsagePricingModel::PerUnit { rate: dec!(0.25) });
        let days = days_of(&window, &[dec!(4), dec!(0), dec!(8)]);

        assert_eq!(amounts(&days), vec![100, 0, 200]);
        assert_eq!(period_price(&window, dec!(12)), 300);
    }

    #[test]
    fn days_starting_after_the_window_count_from_the_usage_before_them() {
        let window = june_window(UsagePricingModel::Tiered {
            tiers: two_tiers(),
            block_size: None,
        });
        let cuts = day_cuts(midnight(3), midnight(4));
        let closings = running_usage(&usage(dec!(90)), &[point(3, dec!(20), None)], &cuts);

        let days = cumulative_daily_costs(
            &window,
            &PinnedPrices::default(),
            Some((midnight(3), usage(dec!(90)))),
            closings,
        )
        .unwrap();

        // 90 units before June 3; its 20 are 10 at 1.00 and 10 at 0.50.
        assert_eq!(
            days,
            vec![DailyCost {
                date: june(3),
                quantity: dec!(20),
                amount_minor: 1_500,
            }]
        );
    }

    #[test]
    fn running_usage_accumulates_each_group_apart() {
        let cuts = day_cuts(midnight(1), midnight(4));
        let opening = vec![GroupedUsageData {
            value: dec!(5),
            dimensions: HashMap::from([("region".to_owned(), "eu".to_owned())]),
        }];
        let points = [
            point(1, dec!(1), Some("eu")),
            point(1, dec!(2), Some("us")),
            point(3, dec!(4), Some("eu")),
        ];

        let closings = running_usage(&opening, &points, &cuts);

        let by_region = |closing: &Closing| {
            let mut regions = closing
                .usage
                .iter()
                .map(|group| (group.dimensions["region"].clone(), group.value))
                .collect::<Vec<_>>();
            regions.sort();
            regions
        };
        assert_eq!(closings.len(), 3);
        let expected_day_1 = vec![("eu".to_owned(), dec!(6)), ("us".to_owned(), dec!(2))];
        assert_eq!(by_region(&closings[0]), expected_day_1);
        assert_eq!(by_region(&closings[1]), expected_day_1);
        assert_eq!(
            by_region(&closings[2]),
            vec![("eu".to_owned(), dec!(10)), ("us".to_owned(), dec!(2))]
        );
    }

    #[test]
    fn days_are_cut_at_utc_midnight_and_at_the_window_end() {
        let noon = june(1).and_hms_opt(12, 0, 0).unwrap();
        let morning = june(3).and_hms_opt(6, 0, 0).unwrap();

        assert_eq!(
            day_cuts(noon, morning),
            vec![
                DayCut {
                    date: june(1),
                    until: midnight(2),
                },
                DayCut {
                    date: june(2),
                    until: midnight(3),
                },
                DayCut {
                    date: june(3),
                    until: morning,
                },
            ]
        );
        assert!(day_cuts(midnight(2), midnight(2)).is_empty());
        assert_eq!(day_after(midnight(3)), june(3));
        assert_eq!(day_after(morning), june(4));
    }

    #[test]
    fn the_previous_period_is_what_the_invoice_opening_the_current_one_bills() {
        let (subscription, other) = (SubscriptionId::new(), SubscriptionId::new());
        let may_1 = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        let cycle = |subscription_id, anchor, cycle_index| RecurringInvoiceCycle {
            subscription_id,
            anchor,
            cycle_index,
            invoice_type: diesel_models::enums::InvoiceType::Recurring,
        };
        // A final invoice shares its period's anchor; it does not open the period.
        let final_one = RecurringInvoiceCycle {
            invoice_type: diesel_models::enums::InvoiceType::Final,
            ..cycle(subscription, june(1), Some(7))
        };
        let cycles = vec![
            cycle(subscription, may_1, Some(4)),
            cycle(other, june(1), Some(9)),
            final_one,
            cycle(subscription, june(1), None),
            cycle(subscription, june(1), Some(5)),
        ];

        assert_eq!(opening_cycle(&cycles, subscription, june(1)), Some(5));
        assert_eq!(opening_cycle(&cycles, subscription, may_1), Some(4));
        assert_eq!(opening_cycle(&cycles, other, may_1), None);
        assert_eq!(opening_cycle(&cycles, subscription, june(15)), None);
    }
}
