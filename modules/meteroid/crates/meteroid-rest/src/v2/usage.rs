//! Usage (ADR-0013 §14): what a customer or a subscription used over a period by metric, its
//! cost over time and its raw events, a customer's usage no subscription bills, and the tenant's
//! usage summary — each on the store service v1 reads it with (`fetch_usage_summary` through
//! v1's `fetch_usage_for_metrics`, the current period's running total through v1's cache,
//! `usage_costs`, `list_metric_events_with_cost`, `unattributed_usage`).
//!
//! Under the resource it is about: `/customers/{id_or_alias}/usage…`,
//! `/subscriptions/{id_or_external_id}/usage…`, and `/usage/summary` for the tenant. Metrics are
//! named by code, quantities are decimal strings, costs are integers in minor units next to
//! their currency or credit unit; an event's cost, a share of a minor unit, is a decimal string
//! in the major unit.
use crate::AppState;
use crate::usage::model::{UsageResponse, sum_costs};
use crate::usage::router::{current_period_usage, fetch_usage_for_metrics};
use crate::usage_costs::model::{UsageCost as V1Cost, UsageCostUnit, decode_cursor, encode_cursor};
use crate::v2::documents::filter_not_found;
use crate::v2::extract::{Path, Query};
use crate::v2::ids;
use crate::v2::pagination::PageQuery;
use crate::v2::problem::Problem;
use crate::v2::refs::{CreditUnitRef, MetricRef, ReferenceIds, Refs, SubscriptionRef};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime};
use common_domain::ids::{CreditUnitId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::constants::Currencies;
use meteroid_store::domain::usage_costs::{
    CostGranularity, CostUnit, MetricRef as MetricNamed, UsageCost as DomainCost, UsageOwner,
};
use meteroid_store::domain::{BillableMetric, UsagePeriod, unattributed_default_range};
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::credit_units::CreditUnitsInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// The longest range a cost or an event read covers, as the store bounds it.
const MAX_COST_RANGE_DAYS: i64 = 93;
/// The longest range a plain usage read covers (a customer's, a subscription's, the tenant's,
/// the unattributed): a year and a day, as the unattributed read was bounded already.
const MAX_USAGE_RANGE_DAYS: i64 = 366;

// ── Model ───────────────────────────────────────────────────────────

/// An amount in minor units of its currency, or of its credit unit for usage priced in credit.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct UsageAmount {
    pub amount: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// The credit unit; what credit leaves uncovered is invoiced in money, not counted here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<CreditUnitRef>,
}

/// The usage of one group of a metric's dimensions.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct UsageGroup {
    pub dimensions: BTreeMap<String, String>,
    #[schema(value_type = String, format = "decimal")]
    pub quantity: Decimal,
}

#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct MetricUsage {
    pub metric: MetricRef,
    pub name: String,
    #[schema(value_type = String, format = "decimal")]
    pub quantity: Decimal,
    pub groups: Vec<UsageGroup>,
    /// Beta, on a subscription's current period only: what the usage adds so far to its next
    /// invoice, before discounts and credits; empty while it bills nothing (a free trial).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub costs: Option<Vec<UsageAmount>>,
}

/// Usage by metric over `[from, to)`.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
#[schema(as = UsageReport)]
pub struct Usage {
    pub from: ApiDateTime,
    pub to: ApiDateTime,
    pub metrics: Vec<MetricUsage>,
    /// Beta, on a subscription's current period only: when `costs` and `totals` were computed;
    /// the reading is shared for up to 30 seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computed_at: Option<ApiDateTime>,
    /// Beta: the metrics' costs added up, per currency or credit unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub totals: Option<Vec<UsageAmount>>,
}

/// How long a cost bucket is.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    Hour,
    Day,
    Month,
}

impl From<Granularity> for CostGranularity {
    fn from(granularity: Granularity) -> Self {
        match granularity {
            Granularity::Hour => CostGranularity::Hour,
            Granularity::Day => CostGranularity::Day,
            Granularity::Month => CostGranularity::Month,
        }
    }
}

impl From<CostGranularity> for Granularity {
    fn from(granularity: CostGranularity) -> Self {
        match granularity {
            CostGranularity::Hour => Granularity::Hour,
            CostGranularity::Day => Granularity::Day,
            CostGranularity::Month => Granularity::Month,
        }
    }
}

/// A bucket of usage and what it costs, priced as its invoice line prices it.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct CostBucket {
    pub start: ApiDateTime,
    /// The value of `group_by`; absent when not grouped, or for usage without the property.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[schema(value_type = String, format = "decimal")]
    pub quantity: Decimal,
    /// Rounded to the minor unit, so a period's buckets add up to what it invoices. Absent when
    /// the aggregation cannot be priced per bucket.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<UsageAmount>,
    /// False when the price depends on the whole period's usage (tiers, volume, packages,
    /// levels): the cost is then what the bucket adds to the period's price so far.
    pub exact: bool,
}

/// A metric's usage and cost per bucket over `[from, to)`, cut at UTC boundaries, within the
/// billing periods pricing it.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct UsageCosts {
    pub metric: MetricRef,
    pub granularity: Granularity,
    pub from: ApiDateTime,
    pub to: ApiDateTime,
    pub buckets: Vec<CostBucket>,
}

/// An event's cost at the price in force, unrounded: a decimal string in the currency's (or the
/// credit unit's) major unit.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct EventCost {
    #[schema(value_type = String, format = "decimal")]
    pub amount: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<CreditUnitRef>,
}

#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct UsageEvent {
    /// The id the event was ingested with.
    pub id: String,
    pub timestamp: ApiDateTime,
    pub properties: BTreeMap<String, String>,
    /// The billed units it adds; absent when the aggregation cannot be split per event, or the
    /// event does not count.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub quantity: Option<Decimal>,
    /// Per-unit prices and rates by dimension only: absent for prices depending on the whole
    /// period's usage, and for a metric whose unit conversion rounds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<EventCost>,
}

crate::v2_list!(UsageEventList, UsageEvent);

/// A stretch of a metric's usage no subscription bills.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct UnattributedMetricUsage {
    pub metric: MetricRef,
    pub name: String,
    pub from: ApiDateTime,
    pub to: ApiDateTime,
    #[schema(value_type = String, format = "decimal")]
    pub quantity: Decimal,
    /// The subscriptions pricing the metric then, whose usage filters leave it out.
    pub subscriptions: Vec<SubscriptionRef>,
}

/// A customer's usage over `[from, to)` that no subscription bills.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct UnattributedUsage {
    pub from: ApiDateTime,
    pub to: ApiDateTime,
    pub metrics: Vec<UnattributedMetricUsage>,
}

// ── Queries ─────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams, Debug, Clone)]
#[into_params(parameter_in = Query)]
pub struct UsageQuery {
    /// The start, inclusive: a date (midnight UTC) or an RFC 3339 date-time.
    pub from: String,
    /// The end, exclusive: a date or an RFC 3339 date-time, at most 366 days after `from`.
    pub to: String,
    /// Only this metric, by code (or id).
    pub metric: Option<String>,
}

#[derive(Deserialize, IntoParams, Debug, Clone)]
#[into_params(parameter_in = Query)]
pub struct SubscriptionUsageQuery {
    /// The start, inclusive: a date or an RFC 3339 date-time; the current period's start when
    /// left out.
    pub from: Option<String>,
    /// The end, exclusive, at most 366 days after `from`; the current period's end when left
    /// out.
    pub to: Option<String>,
    /// Only this metric, by code (or id).
    pub metric: Option<String>,
}

#[derive(Deserialize, IntoParams, Debug, Clone)]
#[into_params(parameter_in = Query)]
pub struct UsageCostQuery {
    /// The metric, by code (or id).
    pub metric: String,
    pub granularity: Granularity,
    /// The start, inclusive: a date or an RFC 3339 date-time.
    pub from: String,
    /// The end, exclusive; at most 93 days after `from`.
    pub to: String,
    /// An event property to split the usage by.
    pub group_by: Option<String>,
}

#[derive(Deserialize, IntoParams, Debug, Clone)]
#[into_params(parameter_in = Query)]
pub struct UsageEventQuery {
    /// The metric, by code (or id).
    pub metric: String,
    /// The start, inclusive: a date or an RFC 3339 date-time.
    pub from: String,
    /// The end, exclusive; at most 93 days after `from`.
    pub to: String,
    /// How many events, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
}

#[derive(Deserialize, IntoParams, Debug, Clone)]
#[into_params(parameter_in = Query)]
pub struct UnattributedQuery {
    /// The start, inclusive; the first day of the current month (UTC) when left out.
    pub from: Option<String>,
    /// The end, exclusive, at most 366 days after `from`; the first day of the next month when
    /// left out.
    pub to: Option<String>,
}

// ── Reading ─────────────────────────────────────────────────────────

/// A moment a query names: a date is its midnight UTC.
fn moment(raw: &str, parameter: &str) -> Result<NaiveDateTime, Problem> {
    let raw = raw.trim();
    if let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Ok(date.and_time(NaiveTime::MIN));
    }
    DateTime::parse_from_rfc3339(raw)
        .map(|at| at.naive_utc())
        .map_err(|_| {
            Problem::field(
                parameter,
                "invalid",
                "A date (2026-12-01) or an RFC 3339 date-time (2026-12-01T00:00:00Z)",
            )
        })
}

/// `[from, to)`, `to` after `from`.
fn range(from: &str, to: &str) -> Result<(NaiveDateTime, NaiveDateTime), Problem> {
    let (from, to) = (moment(from, "from")?, moment(to, "to")?);
    if to <= from {
        return Err(Problem::field("to", "invalid", "`to` is after `from`"));
    }
    Ok((from, to))
}

/// Refuses at `to` a range `to` does not end after `from`, or longer than `days`.
fn bounded(from: NaiveDateTime, to: NaiveDateTime, days: i64) -> Result<(), Problem> {
    if to <= from {
        return Err(Problem::field("to", "invalid", "`to` is after `from`"));
    }
    if to - from > Duration::days(days) {
        return Err(Problem::field(
            "to",
            "invalid",
            format!("The range is {days} days at most"),
        ));
    }
    Ok(())
}

/// A range a plain usage read covers: 366 days at most.
fn usage_range(from: &str, to: &str) -> Result<(NaiveDateTime, NaiveDateTime), Problem> {
    let (from, to) = range(from, to)?;
    bounded(from, to, MAX_USAGE_RANGE_DAYS)?;
    Ok((from, to))
}

/// A range a cost or event read covers: 93 days at most.
fn cost_range(from: &str, to: &str) -> Result<(NaiveDateTime, NaiveDateTime), Problem> {
    let (from, to) = range(from, to)?;
    bounded(from, to, MAX_COST_RANGE_DAYS)?;
    Ok((from, to))
}

/// The metric a `metric` parameter names, refused at it when none.
async fn named_metric(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<BillableMetric, Problem> {
    ids::metric(state, tenant_id, raw)
        .await
        .map_err(filter_not_found("metric", raw))
}

/// The cost reads are gated by the organization's plan, and rate limited.
async fn open_costs(state: &AppState, auth: &AuthorizedAsTenant) -> Result<(), Problem> {
    if !crate::usage_costs::router::usage_costs_open(state, auth).await {
        return Err(Problem::feature_not_available(
            "Usage costs are not enabled for this organization",
        ));
    }
    crate::rate_limit::rate_limit_usage_costs(state, auth).await?;
    Ok(())
}

/// The minor-unit exponents of the currencies and credit units a response names.
struct Denominations<'a> {
    state: &'a AppState,
    tenant_id: TenantId,
    credit_units: HashMap<String, (u8, CreditUnitId)>,
}

impl<'a> Denominations<'a> {
    fn new(state: &'a AppState, tenant_id: TenantId) -> Self {
        Denominations {
            state,
            tenant_id,
            credit_units: HashMap::new(),
        }
    }

    /// `amount`, in the major unit of `code`, as an integer of its minor units.
    async fn minor(
        &mut self,
        amount: Decimal,
        code: &str,
        credit_unit: bool,
    ) -> Result<UsageAmount, Problem> {
        let (precision, unit) = if credit_unit {
            let (precision, id) = match self.credit_units.get(code) {
                Some(found) => *found,
                None => {
                    let unit = self
                        .state
                        .store
                        .find_credit_unit_by_code(self.tenant_id, code)
                        .await?;
                    self.credit_units
                        .insert(code.to_owned(), (unit.precision, unit.id));
                    (unit.precision, unit.id)
                }
            };
            (
                precision,
                Some(CreditUnitRef {
                    id,
                    code: code.to_owned(),
                }),
            )
        } else {
            (
                Currencies::resolve_currency_precision(code).unwrap_or(2),
                None,
            )
        };
        let scaled = amount * Decimal::from(10_i64.pow(u32::from(precision)));
        Ok(UsageAmount {
            amount: scaled
                .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
                .to_i64()
                .ok_or_else(Problem::internal)?,
            currency: (!credit_unit).then(|| code.to_owned()),
            credit_unit: unit,
        })
    }

    async fn of_v1(&mut self, cost: &V1Cost) -> Result<UsageAmount, Problem> {
        self.minor(
            cost.amount,
            &cost.currency,
            cost.unit == UsageCostUnit::CreditUnit,
        )
        .await
    }

    async fn of_domain(&mut self, cost: &DomainCost) -> Result<UsageAmount, Problem> {
        self.minor(
            cost.amount,
            &cost.currency,
            cost.unit == CostUnit::CreditUnit,
        )
        .await
    }

    async fn all(&mut self, costs: &[V1Cost]) -> Result<Vec<UsageAmount>, Problem> {
        let mut out = Vec::with_capacity(costs.len());
        for cost in costs {
            out.push(self.of_v1(cost).await?);
        }
        Ok(out)
    }
}

fn instant(date: NaiveDate) -> ApiDateTime {
    date.and_time(NaiveTime::MIN).into()
}

/// v1's reading as v2 renders it, over `[from, to)`.
async fn render(
    state: &AppState,
    tenant_id: TenantId,
    reading: &UsageResponse,
    from: ApiDateTime,
    to: ApiDateTime,
) -> Result<Usage, Problem> {
    let mut denominations = Denominations::new(state, tenant_id);
    let ids = ReferenceIds {
        metrics: reading
            .usage
            .iter()
            .map(|metric| metric.metric_id)
            .collect(),
        ..ReferenceIds::default()
    };
    let refs = Refs::load(&state.store, tenant_id, &ids).await?;
    let mut metrics = Vec::with_capacity(reading.usage.len());
    for metric in &reading.usage {
        let costs = match &metric.costs {
            Some(costs) => Some(denominations.all(costs).await?),
            None => None,
        };
        metrics.push(MetricUsage {
            metric: refs.metric(metric.metric_id),
            name: metric.metric_name.clone(),
            quantity: metric.total_value.normalize(),
            groups: metric
                .grouped_usage
                .iter()
                .map(|group| UsageGroup {
                    dimensions: group
                        .dimensions
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                    quantity: group.value.normalize(),
                })
                .collect(),
            costs,
        });
    }
    let totals = match &reading.totals {
        Some(totals) => Some(denominations.all(totals).await?),
        None => None,
    };
    Ok(Usage {
        from,
        to,
        metrics,
        computed_at: reading.computed_at,
        totals,
    })
}

/// Each of `metrics`' usage over `[from, to)`, the customer's or (without one) the tenant's.
async fn usage_over(
    state: &AppState,
    tenant_id: TenantId,
    customer_id: Option<common_domain::ids::CustomerId>,
    metrics: &[BillableMetric],
    from: NaiveDateTime,
    to: NaiveDateTime,
) -> Result<Usage, Problem> {
    let usage = fetch_usage_for_metrics(
        state,
        &tenant_id,
        customer_id.as_ref(),
        metrics,
        UsagePeriod {
            start: from,
            end: to,
        },
    )
    .await?;
    let reading = UsageResponse {
        period_start: from.date(),
        period_end: to.date(),
        usage,
        computed_at: None,
        totals: None,
    };
    render(state, tenant_id, &reading, from.into(), to.into()).await
}

/// The metrics a tenant-wide or customer read covers: the one named, else the live ones.
async fn metrics_read(
    state: &AppState,
    tenant_id: TenantId,
    metric: Option<&str>,
) -> Result<Vec<BillableMetric>, Problem> {
    match metric {
        Some(raw) => Ok(vec![named_metric(state, tenant_id, raw).await?]),
        None => Ok(state.store.list_active_billable_metrics(tenant_id).await?),
    }
}

async fn costs(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    owner: UsageOwner,
    query: UsageCostQuery,
) -> Result<UsageCosts, Problem> {
    let (from, to) = cost_range(&query.from, &query.to)?;
    // Checked first, so a store refusal not found is the customer's or subscription's.
    let metric = named_metric(state, auth.tenant_id, &query.metric).await?;
    let group_by = query
        .group_by
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned);
    if group_by.as_deref().is_some_and(|key| key.len() > 128) {
        return Err(Problem::field(
            "group_by",
            "invalid",
            "A property name, 128 bytes at most",
        ));
    }
    let rows = state
        .services
        .usage_costs(
            auth.tenant_id,
            owner,
            MetricNamed::Id(metric.id),
            group_by,
            query.granularity.into(),
            from,
            to,
        )
        .await
        .map_err(|error| Problem::at("metric", error))?;
    let mut denominations = Denominations::new(state, auth.tenant_id);
    let mut buckets = Vec::with_capacity(rows.len());
    for row in rows {
        let cost = match &row.cost {
            Some(cost) => Some(denominations.of_domain(cost).await?),
            None => None,
        };
        buckets.push(CostBucket {
            start: row.bucket_start.into(),
            group: row.group,
            quantity: row.quantity.normalize(),
            cost,
            exact: row.exact,
        });
    }
    Ok(UsageCosts {
        metric: MetricRef {
            id: metric.id,
            code: metric.catalog_code,
        },
        granularity: query.granularity,
        from: from.into(),
        to: to.into(),
        buckets,
    })
}

async fn events(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    owner: UsageOwner,
    query: UsageEventQuery,
) -> Result<UsageEventList, Problem> {
    let (from, to) = cost_range(&query.from, &query.to)?;
    let metric = named_metric(state, auth.tenant_id, &query.metric).await?;
    let paging = PageQuery {
        limit: query.limit,
        after: None,
    };
    let limit = paging.limit()?;
    let after = query
        .after
        .as_deref()
        .map(str::trim)
        .filter(|after| !after.is_empty())
        .map(|raw| {
            decode_cursor(raw)
                .ok_or_else(|| Problem::field("after", "invalid", "Not a cursor of this list"))
        })
        .transpose()?;
    let page = state
        .services
        .list_metric_events_with_cost(
            auth.tenant_id,
            owner,
            MetricNamed::Id(metric.id),
            from,
            to,
            after,
            u32::try_from(limit).unwrap_or(u32::MAX),
        )
        .await
        .map_err(|error| Problem::at("metric", error))?;
    let next_cursor = page.next.as_ref().map(encode_cursor);
    let ids = ReferenceIds {
        credit_unit_codes: page
            .events
            .iter()
            .filter_map(|item| item.cost.as_ref())
            .filter(|cost| cost.unit == CostUnit::CreditUnit)
            .map(|cost| cost.currency.clone())
            .collect(),
        ..ReferenceIds::default()
    };
    let refs = Refs::load(&state.store, auth.tenant_id, &ids).await?;
    Ok(UsageEventList {
        has_more: next_cursor.is_some(),
        next_cursor,
        data: page
            .events
            .into_iter()
            .map(|item| UsageEvent {
                id: item.event.id,
                timestamp: item.event.timestamp.into(),
                properties: item.event.properties.into_iter().collect(),
                quantity: item.quantity.map(|quantity| quantity.normalize()),
                cost: item.cost.map(|cost| {
                    let credit = cost.unit == CostUnit::CreditUnit;
                    EventCost {
                        amount: cost.amount.normalize(),
                        currency: (!credit).then(|| cost.currency.clone()),
                        credit_unit: credit
                            .then(|| refs.credit_unit_coded(&cost.currency))
                            .flatten(),
                    }
                }),
            })
            .collect(),
    })
}

// ── Customers ───────────────────────────────────────────────────────

/// Get a customer's usage
///
/// Its usage of each live metric (or of `metric`) over `[from, to)`, across its subscriptions,
/// split by the metric's dimensions; 366 days at most. Counts towards the usage cost reads' rate
/// limit.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/usage",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        UsageQuery,
    ),
    responses(
        (status = 200, description = "The customer's usage by metric", body = Usage),
        (status = 404, description = "The customer is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A parameter is refused: an unknown metric, a range ending before it starts or longer than 366 days…", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many usage reads", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_customer_usage(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<Usage>, Problem> {
    crate::rate_limit::rate_limit_usage_costs(&state, &auth).await?;
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let (from, to) = usage_range(&query.from, &query.to)?;
    let metrics = metrics_read(&state, auth.tenant_id, query.metric.as_deref()).await?;
    Ok(Json(
        usage_over(
            &state,
            auth.tenant_id,
            Some(customer.id),
            &metrics,
            from,
            to,
        )
        .await?,
    ))
}

/// Get a customer's usage costs
///
/// One metric's usage and cost per hour, day or month, split by an event property when asked,
/// summed across the subscriptions billing it. Per-unit prices and rates by dimension cost
/// exactly; a price depending on the whole period's usage gives what each bucket adds to it
/// (`exact: false`). At most 93 days.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/usage/costs",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        UsageCostQuery,
    ),
    responses(
        (status = 200, description = "Usage and cost per bucket", body = UsageCosts),
        (status = 403, description = "Usage costs are not enabled for this organization", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "The customer is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A parameter is refused", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many usage cost reads", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_customer_usage_costs(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
    Query(query): Query<UsageCostQuery>,
) -> Result<Json<UsageCosts>, Problem> {
    open_costs(&state, &auth).await?;
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    Ok(Json(
        costs(&state, &auth, UsageOwner::Customer(customer.id), query).await?,
    ))
}

/// List a customer's usage events
///
/// One metric's events, oldest first, paged by cursor, each with the units it adds and its cost
/// at the price in force. At most 93 days.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/usage/events",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        UsageEventQuery,
    ),
    responses(
        (status = 200, description = "A page of events", body = UsageEventList),
        (status = 403, description = "Usage costs are not enabled for this organization", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "The customer is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A parameter is refused", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many usage cost reads", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_customer_usage_events(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
    Query(query): Query<UsageEventQuery>,
) -> Result<Json<UsageEventList>, Problem> {
    open_costs(&state, &auth).await?;
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    Ok(Json(
        events(&state, &auth, UsageOwner::Customer(customer.id), query).await?,
    ))
}

/// Get a customer's unattributed usage
///
/// Its usage no subscription bills: where the subscriptions pricing a metric all have a usage
/// filter, the events matching none of them.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/usage/unattributed",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        UnattributedQuery,
    ),
    responses(
        (status = 200, description = "The usage no subscription bills", body = UnattributedUsage),
        (status = 404, description = "The customer is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A parameter is refused", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many usage reads", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_unattributed_usage(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
    Query(query): Query<UnattributedQuery>,
) -> Result<Json<UnattributedUsage>, Problem> {
    crate::rate_limit::rate_limit_usage_costs(&state, &auth).await?;
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let (default_from, default_to) = unattributed_default_range(common_clock::now().date_naive());
    let from = match query.from.as_deref() {
        Some(raw) => moment(raw, "from")?,
        None => default_from.and_time(NaiveTime::MIN),
    };
    let to = match query.to.as_deref() {
        Some(raw) => moment(raw, "to")?,
        None => default_to.and_time(NaiveTime::MIN),
    };
    bounded(from, to, MAX_USAGE_RANGE_DAYS)?;
    let usage = state
        .services
        .unattributed_usage(auth.tenant_id, customer.id, from, to)
        .await
        .map_err(|error| Problem::at("to", error))?;
    let ids = ReferenceIds {
        subscriptions: usage
            .iter()
            .flat_map(|usage| usage.subscription_ids.iter().copied())
            .collect(),
        ..ReferenceIds::default()
    };
    let refs = Refs::load(&state.store, auth.tenant_id, &ids).await?;
    Ok(Json(UnattributedUsage {
        from: from.into(),
        to: to.into(),
        metrics: usage
            .into_iter()
            .map(|usage| UnattributedMetricUsage {
                metric: MetricRef {
                    id: usage.metric.id,
                    code: usage.metric.catalog_code,
                },
                name: usage.metric.name,
                from: usage.from.into(),
                to: usage.until.into(),
                quantity: usage.quantity.normalize(),
                subscriptions: usage
                    .subscription_ids
                    .into_iter()
                    .map(|id| refs.subscription(id))
                    .collect(),
            })
            .collect(),
    }))
}

// ── Subscriptions ───────────────────────────────────────────────────

/// Get a subscription's usage
///
/// Its usage of each metric it bills over `[from, to)`. Left out, the range is its current
/// period, and the reading then carries its running total (beta): what each metric's usage adds
/// so far to the next invoice, and their sum, before discounts and credits, shared for up to 30
/// seconds (`computed_at`). Counts towards the usage cost reads' rate limit.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/usage",
    params(
        ("id_or_external_id" = String, Path, description = "Subscription id or external id"),
        SubscriptionUsageQuery,
    ),
    responses(
        (status = 200, description = "The subscription's usage by metric", body = Usage),
        (status = 404, description = "The subscription is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A parameter is refused", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many usage cost reads", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_subscription_usage(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
    Query(query): Query<SubscriptionUsageQuery>,
) -> Result<Json<Usage>, Problem> {
    crate::rate_limit::rate_limit_usage_costs(&state, &auth).await?;
    let tenant_id = auth.tenant_id;
    let subscription_id = ids::subscription_id(&state, tenant_id, &id_or_external_id).await?;
    let metric = match query.metric.as_deref() {
        Some(raw) => Some(named_metric(&state, tenant_id, raw).await?.id),
        None => None,
    };

    if query.from.is_none() && query.to.is_none() {
        let current = state
            .subscription_usage
            .try_get_with(
                (tenant_id, subscription_id),
                current_period_usage(&state, tenant_id, subscription_id),
            )
            .await
            .map_err(|error| Problem::from((*error).clone()))?;
        if current.computed_at.is_none() {
            state
                .subscription_usage
                .invalidate(&(tenant_id, subscription_id))
                .await;
        }
        let mut reading = (*current).clone();
        if let Some(id) = metric {
            reading.usage.retain(|usage| usage.metric_id == id);
            if reading.totals.is_some() {
                reading.totals = Some(sum_costs(
                    reading
                        .usage
                        .iter()
                        .flat_map(|usage| usage.costs.iter().flatten()),
                ));
            }
        }
        if !crate::usage_costs::router::usage_costs_open(&state, &auth).await {
            reading = reading.without_costs();
        }
        let (from, to) = (instant(reading.period_start), instant(reading.period_end));
        return Ok(Json(render(&state, tenant_id, &reading, from, to).await?));
    }

    let details = state
        .store
        .get_subscription_details(tenant_id, subscription_id)
        .await?;
    let from = match query.from.as_deref() {
        Some(raw) => moment(raw, "from")?,
        None => details
            .subscription
            .current_period_start
            .and_time(NaiveTime::MIN),
    };
    let to = match query.to.as_deref() {
        Some(raw) => moment(raw, "to")?,
        None => details
            .subscription
            .current_period_end
            .unwrap_or_else(|| common_clock::now().date_naive() + chrono::Duration::days(1))
            .and_time(NaiveTime::MIN),
    };
    bounded(from, to, MAX_USAGE_RANGE_DAYS)?;
    let metrics: Vec<BillableMetric> = details
        .metrics
        .into_iter()
        .filter(|billed| metric.is_none_or(|id| billed.id == id))
        .collect();
    Ok(Json(
        usage_over(
            &state,
            tenant_id,
            Some(details.subscription.customer_id),
            &metrics,
            from,
            to,
        )
        .await?,
    ))
}

/// Get a subscription's usage costs
///
/// As a customer's, counting only the events the subscription bills (its usage filter) and only
/// its own billing periods. At most 93 days.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/usage/costs",
    params(
        ("id_or_external_id" = String, Path, description = "Subscription id or external id"),
        UsageCostQuery,
    ),
    responses(
        (status = 200, description = "Usage and cost per bucket", body = UsageCosts),
        (status = 403, description = "Usage costs are not enabled for this organization", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "The subscription is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A parameter is refused, or a metric the subscription does not bill", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many usage cost reads", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_subscription_usage_costs(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
    Query(query): Query<UsageCostQuery>,
) -> Result<Json<UsageCosts>, Problem> {
    open_costs(&state, &auth).await?;
    let subscription_id = ids::subscription_id(&state, auth.tenant_id, &id_or_external_id).await?;
    Ok(Json(
        costs(
            &state,
            &auth,
            UsageOwner::Subscription(subscription_id),
            query,
        )
        .await?,
    ))
}

/// List a subscription's usage events
///
/// The events of one metric the subscription bills, oldest first, paged by cursor: with a usage
/// filter, the customer's events matching it; without, those no other subscription's filter
/// claims. At most 93 days.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/usage/events",
    params(
        ("id_or_external_id" = String, Path, description = "Subscription id or external id"),
        UsageEventQuery,
    ),
    responses(
        (status = 200, description = "A page of events", body = UsageEventList),
        (status = 403, description = "Usage costs are not enabled for this organization", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "The subscription is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A parameter is refused, or a metric the subscription does not bill", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many usage cost reads", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_subscription_usage_events(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
    Query(query): Query<UsageEventQuery>,
) -> Result<Json<UsageEventList>, Problem> {
    open_costs(&state, &auth).await?;
    let subscription_id = ids::subscription_id(&state, auth.tenant_id, &id_or_external_id).await?;
    Ok(Json(
        events(
            &state,
            &auth,
            UsageOwner::Subscription(subscription_id),
            query,
        )
        .await?,
    ))
}

// ── Tenant ──────────────────────────────────────────────────────────

/// Get the usage summary
///
/// The tenant's usage of each live metric (or of `metric`) over `[from, to)`, all customers
/// together; 366 days at most. Counts towards the usage cost reads' rate limit.
#[utoipa::path(
    get,
    tag = "Usage",
    path = "/api/v2/usage/summary",
    params(UsageQuery),
    responses(
        (status = 200, description = "The tenant's usage by metric", body = Usage),
        (status = 422, description = "A parameter is refused: a range longer than 366 days…", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many usage reads", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_usage_summary(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<Usage>, Problem> {
    crate::rate_limit::rate_limit_usage_costs(&state, &auth).await?;
    let (from, to) = usage_range(&query.from, &query.to)?;
    let metrics = metrics_read(&state, auth.tenant_id, query.metric.as_deref()).await?;
    Ok(Json(
        usage_over(&state, auth.tenant_id, None, &metrics, from, to).await?,
    ))
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_customer_usage))
        .routes(routes!(get_customer_usage_costs))
        .routes(routes!(list_customer_usage_events))
        .routes(routes!(get_unattributed_usage))
        .routes(routes!(get_subscription_usage))
        .routes(routes!(get_subscription_usage_costs))
        .routes(routes!(list_subscription_usage_events))
        .routes(routes!(get_usage_summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moment_is_a_date_or_a_date_time() {
        assert_eq!(
            moment("2026-12-01", "from").unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 1)
                .unwrap()
                .and_time(NaiveTime::MIN)
        );
        assert_eq!(
            moment("2026-12-01T10:00:00+01:00", "from").unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 1)
                .unwrap()
                .and_hms_opt(9, 0, 0)
                .unwrap()
        );
        assert_eq!(
            moment("yesterday", "from").unwrap_err().errors[0].path,
            "from"
        );
        assert_eq!(
            range("2026-12-02", "2026-12-01").unwrap_err().errors[0].path,
            "to"
        );
        assert!(cost_range("2026-01-01", "2026-06-01").is_err());
        assert!(usage_range("2026-01-01", "2027-01-01").is_ok());
        assert_eq!(
            usage_range("2025-01-01", "2027-01-01").unwrap_err().errors[0].path,
            "to"
        );
    }
}
