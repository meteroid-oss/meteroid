//! The usage windows an invoice bills and what their usage prices at, as invoicing derives them:
//! the upcoming invoice of each subscription, a draft's or a final invoice's own windows.
use crate::StoreResult;
use crate::constants::Currencies;
use crate::domain::{BillableMetric, Period, SubscriptionDetails, SubscriptionFee, UsagePeriod};
use crate::errors::StoreError;
use crate::repositories::credit::customer_credit::normalized_currency;
use crate::repositories::subscriptions::SubscriptionInterface;
use crate::services::clients::usage::{GroupedUsageData, UsageData};
use crate::services::credit::invalid;
use crate::services::credit::unit_wallets::OverageRates;
use crate::services::invoice_lines::component::{
    PinnedPrices, capacity_overage_lines, convert_usage_units, rated_usage_lines, usage_lines,
};
use crate::services::invoice_lines::invoice_lines::{BilledSpan, MeteredFee};
use crate::services::invoice_lines::invoice_lines::{build_tax_engine, rates_of};
use crate::services::invoice_lines::tax_inputs::{customer_for_tax, product_tax_config_with_conn};
use crate::services::invoices::upcoming::upcoming_invoice_details;
use crate::store::{PgConn, Store};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use common_domain::ids::{
    BillableMetricId, CreditUnitId, CustomerId, PlanVersionId, ProductId, SubscriptionAddOnId,
    SubscriptionId, SubscriptionPriceComponentId, TenantId,
};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::{Report, ResultExt};
use std::collections::HashSet;

/// One metric's usage window, as the upcoming invoice of a subscription bills it.
#[derive(Debug, Clone)]
pub struct PlannedWindow {
    pub subscription_id: SubscriptionId,
    pub metric: BillableMetric,
    pub fee: MeteredFee,
    pub sub_component_id: Option<SubscriptionPriceComponentId>,
    pub sub_add_on_id: Option<SubscriptionAddOnId>,
    /// The product of its component or add-on, which credit restricted to products answers to.
    pub product_id: Option<ProductId>,
    /// Priced tax included, as the subscription quotes.
    pub tax_inclusive: bool,
    /// The taxes its invoice line will levy (rates only), resolved by [`resolve_window_taxes`];
    /// empty when untaxed or not resolved.
    pub taxes: Vec<crate::domain::TaxDetail>,
    pub period: Period,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub currency: String,
    pub precision: u8,
    /// A credit-priced window's money side: the credits its unit wallet leaves uncovered are
    /// billed in the subscription's currency (Studio's overage).
    pub overage: Option<MoneyOverage>,
    /// The rates its usage is rated by dimension at: the entries of the rate card its price
    /// names, or the price's own rates.
    pub card: Option<billing_core::pricing::CardRates>,
}

/// What a credit-priced window bills in money for the credits no unit credit covers.
#[derive(Debug, Clone)]
pub struct MoneyOverage {
    /// The subscription's currency, normalized.
    pub currency: String,
    pub precision: u8,
    /// Currency major units per credit.
    pub rate: rust_decimal::Decimal,
    /// The taxes its invoice line levies (rates only); empty when untaxed or not resolved.
    pub taxes: Vec<crate::domain::TaxDetail>,
}

impl PlannedWindow {
    /// Whether the window bills `currency`, in its own denomination or as money overage.
    pub fn bills_money_in(&self, currency: &str) -> bool {
        self.currency == currency
            || self
                .overage
                .as_ref()
                .is_some_and(|overage| overage.currency == currency)
    }
}

/// A metric's charge: the subscription, the denomination and the product it is billed under.
pub type Charge = (SubscriptionId, String, Option<ProductId>);

/// The subscriptions charging `metric_id`'s usage, each with the denomination it charges it in
/// and the product it bills it under, as the projection prices it (see `Plan::spend_metric_ids`): a credit unit's code for a fee
/// priced in it, else the subscription's currency. Free fees charge nothing.
pub fn charged_denominations(
    windows: &[PlannedWindow],
    metric_id: BillableMetricId,
) -> HashSet<Charge> {
    windows
        .iter()
        .filter(|window| window.metric.id == metric_id && !window.fee.is_free())
        .map(|window| {
            (
                window.subscription_id,
                window.currency.clone(),
                window.product_id,
            )
        })
        .collect()
}

/// The windows the upcoming invoices of `subscriptions` bill, each with its taxes resolved.
pub async fn subscriptions_windows(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscriptions: &[SubscriptionRow],
    now: DateTime<Utc>,
) -> StoreResult<Vec<PlannedWindow>> {
    let mut windows = Vec::new();
    for subscription in subscriptions {
        let details = store
            .get_subscription_details_with_conn(conn, tenant_id, subscription.id)
            .await?;
        let mut subscription_windows = subscription_windows(conn, &details).await?;
        resolve_window_taxes(store, conn, &details, &mut subscription_windows, now).await?;
        windows.extend(subscription_windows);
    }
    Ok(windows)
}

/// [`subscriptions_windows`] of the customer's active subscriptions, of those in `only` when given.
pub(crate) async fn customer_windows(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    only: Option<&HashSet<SubscriptionId>>,
    now: DateTime<Utc>,
) -> StoreResult<Vec<PlannedWindow>> {
    let subscriptions = SubscriptionRow::find_active_by_customer(conn, customer_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .filter(|subscription| only.is_none_or(|only| only.contains(&subscription.id)))
        .collect::<Vec<_>>();
    subscriptions_windows(store, conn, tenant_id, &subscriptions, now).await
}

/// The taxes the invoice of each money window will levy, resolved by the engine on its product as
/// the invoice resolves them; none without an engine or a billing address, as the invoice.
pub(crate) async fn resolve_window_taxes(
    store: &Store,
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    windows: &mut [PlannedWindow],
    now: DateTime<Utc>,
) -> StoreResult<()> {
    let Some(address) = details.customer.billing_address.clone() else {
        return Ok(());
    };
    let Some(engine) = build_tax_engine(&details.invoicing_entity)? else {
        return Ok(());
    };
    let currency = normalized_currency(&details.subscription.currency);
    // Credit-priced windows resolve the rates of their money overage, never of their credits.
    let money = (0..windows.len())
        .filter(|index| windows[*index].bills_money_in(&currency))
        .collect::<Vec<_>>();
    if money.is_empty() {
        return Ok(());
    }
    let config = product_tax_config_with_conn(
        store,
        conn,
        &details.invoicing_entity,
        &money
            .iter()
            .map(|index| windows[*index].product_id)
            .collect::<Vec<_>>(),
    )
    .await?;
    let probes = money
        .iter()
        .map(|index| {
            let (custom_taxes, tax_category) = config.for_product(windows[*index].product_id);
            meteroid_tax::LineItemForTax {
                line_id: index.to_string(),
                amount: RATE_PROBE_MINOR,
                custom_taxes,
                tax_category,
                tax_inclusive: details.subscription.tax_inclusive,
            }
        })
        .collect();
    let resolved = engine
        .calculate_line_items_tax(
            details.subscription.currency.clone(),
            customer_for_tax(&details.customer, &details.invoicing_entity, address),
            details.invoicing_entity.address().into(),
            probes,
            now.date_naive(),
        )
        .await
        .change_context(StoreError::TaxError)?;
    for line in resolved.line_items {
        if let Some(window) = line
            .line_id
            .parse::<usize>()
            .ok()
            .and_then(|index| windows.get_mut(index))
        {
            let rates = rates_of(&line.tax_details);
            if window.currency == currency {
                window.taxes = rates;
            } else if let Some(overage) = window.overage.as_mut() {
                overage.taxes = rates;
            }
        }
    }
    Ok(())
}

/// The amount rates are resolved on; only the rates are kept.
const RATE_PROBE_MINOR: i64 = 1_000_000;

pub async fn subscription_windows(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
) -> StoreResult<Vec<PlannedWindow>> {
    // A free trial's usage is never billed: the invoice ending it opens cycle 0, with no arrears.
    if details.subscription.status == crate::domain::SubscriptionStatusEnum::TrialActive
        && crate::services::subscriptions::in_free_trial_period_tx(conn, &details.subscription)
            .await?
    {
        return Ok(Vec::new());
    }
    let (upcoming, invoice_date) = upcoming_invoice_details(details);
    windows_at(conn, &upcoming, invoice_date, BilledSpan::Period).await
}

/// The windows the invoice of `invoice_date` (keyed as `billed_at` keys it) carries for the
/// subscription, its period's own or its final one (`span`), read from its facts as billing
/// evaluates them (R-084, R-016, R-002).
pub(crate) async fn windows_at(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    invoice_date: NaiveDate,
    span: BilledSpan,
) -> StoreResult<Vec<PlannedWindow>> {
    // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
    Box::pin(async move {
        let subscription = &details.subscription;
        let carried = crate::services::evaluate::windows::invoice_windows(
            conn,
            subscription.tenant_id,
            subscription.id,
            invoice_date,
            span,
        )
        .await?;
        let currency = normalized_currency(&subscription.currency);
        let precision = Currencies::resolve_currency(&currency)
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(format!(
                    "Currency {currency} not found"
                )))
            })?
            .precision;
        let mut planned = Vec::with_capacity(carried.len());
        let mut credit_priced: Vec<(usize, CreditUnitId, String, PlanVersionId)> = Vec::new();
        for window in carried {
            let (fee, credit_unit) = match &window.fee.fee {
                SubscriptionFee::Usage {
                    model, credit_unit, ..
                } => (MeteredFee::Usage(model.clone()), credit_unit.clone()),
                SubscriptionFee::Capacity {
                    included,
                    overage_rate,
                    credit_unit,
                    ..
                } => (
                    MeteredFee::CapacityOverage {
                        included: window
                            .allowance
                            .prorate_decimal(rust_decimal::Decimal::from(*included)),
                        overage_rate: *overage_rate,
                    },
                    credit_unit.clone(),
                ),
                _ => continue,
            };
            if let Some(unit) = &credit_unit {
                credit_priced.push((
                    planned.len(),
                    unit.unit_id,
                    unit.code.clone(),
                    subscription.plan_version_id,
                ));
            }
            let (overage, (window_currency, window_precision)) = match &credit_unit {
                Some(unit) => (
                    Some(MoneyOverage {
                        currency: currency.clone(),
                        precision,
                        rate: unit.rate,
                        taxes: Vec::new(),
                    }),
                    (normalized_currency(&unit.code), unit.precision),
                ),
                None => (None, (currency.clone(), precision)),
            };
            planned.push(PlannedWindow {
                subscription_id: subscription.id,
                metric: window.metric,
                fee,
                sub_component_id: window.fee.sub_component_id,
                sub_add_on_id: window.fee.sub_add_on_id,
                product_id: window.fee.product_id,
                tax_inclusive: subscription.tax_inclusive,
                taxes: Vec::new(),
                period: Period {
                    start: window.days.0,
                    end: window.days.1,
                },
                start: window.read.from.naive_utc(),
                end: window.read.until.naive_utc(),
                currency: window_currency,
                precision: window_precision,
                overage,
                card: None,
            });
        }
        rate_windows(
            conn,
            subscription.tenant_id,
            &subscription.currency,
            &mut planned,
        )
        .await?;
        resolve_overage_rates(conn, details, &currency, &mut planned, credit_priced).await?;
        Ok(planned)
    })
    .await
}

/// The rate of each credit-priced window's money overage (`credit_priced`, by index in `planned`),
/// as the customer's account and the plan version price the credits past the balance.
async fn resolve_overage_rates(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    currency: &str,
    planned: &mut [PlannedWindow],
    credit_priced: Vec<(usize, CreditUnitId, String, PlanVersionId)>,
) -> StoreResult<()> {
    if credit_priced.is_empty() {
        return Ok(());
    }
    let subscription = &details.subscription;
    let unit_ids: Vec<CreditUnitId> = credit_priced.iter().map(|(_, id, ..)| *id).collect();
    let plan_version_ids: Vec<PlanVersionId> =
        credit_priced.iter().map(|(.., version)| *version).collect();
    let mut rates = OverageRates::load(
        conn,
        subscription.tenant_id,
        subscription.customer_id,
        currency,
        &unit_ids,
        &plan_version_ids,
    )
    .await?;
    for (index, unit_id, code, plan_version_id) in credit_priced {
        if let Some(overage) = planned[index].overage.as_mut() {
            overage.rate = rates.rate(unit_id, &code, plan_version_id, overage.rate);
        }
    }
    Ok(())
}

/// Each window rated by dimension gets its rates, a rate card's entries in the subscription's
/// `currency` or its price's own, and reads its metric by the dimensions they key.
async fn rate_windows(
    conn: &mut PgConn,
    tenant_id: common_domain::ids::TenantId,
    currency: &str,
    planned: &mut [PlannedWindow],
) -> StoreResult<()> {
    for window in planned.iter_mut() {
        let MeteredFee::Usage(crate::domain::UsagePricingModel::DimensionRates(rates)) =
            &window.fee
        else {
            continue;
        };
        let card = billing_core::pricing::CardRates {
            on: window.start.date(),
            ..rates.card_rates()
        };
        for dimension in &card.dimensions {
            if !window.metric.rated_dimensions.contains(dimension) {
                window.metric.rated_dimensions.push(dimension.clone());
            }
        }
        window.card = Some(card);
    }
    let card_of = |window: &PlannedWindow| match &window.fee {
        MeteredFee::Usage(crate::domain::UsagePricingModel::RateCard {
            rate_card_id,
            multiplier,
        }) => Some((*rate_card_id, *multiplier)),
        _ => None,
    };
    let ids: Vec<_> = planned
        .iter()
        .filter_map(|w| card_of(w).map(|(id, _)| id))
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let cards =
        crate::services::evaluate::load::card_rates(conn, tenant_id, ids.into_iter(), currency)
            .await?;
    for window in planned.iter_mut() {
        let Some((id, multiplier)) = card_of(window) else {
            continue;
        };
        let Some(card) = cards.get(&id) else {
            continue;
        };
        for dimension in &card.dimensions {
            if !window.metric.rated_dimensions.contains(dimension) {
                window.metric.rated_dimensions.push(dimension.clone());
            }
        }
        window.card = Some(billing_core::pricing::CardRates {
            multiplier: multiplier.unwrap_or(rust_decimal::Decimal::ONE),
            on: window.start.date(),
            ..card.clone()
        });
    }
    Ok(())
}

/// `groups` without the dimensions only the metric's rates key, for a price not rated by them: the
/// metric is read broken down by its rates' keys for every price, and the invoice folds them away
/// for the others (`evaluate::pricer::fee_of`), so a tiered price never bills them apart.
fn folded(window: &PlannedWindow, groups: Vec<GroupedUsageData>) -> Vec<GroupedUsageData> {
    if window.card.is_some() {
        return groups;
    }
    let own = crate::services::usage_grouping::own_dimensions(&window.metric);
    let away: Vec<&String> = window
        .metric
        .rated_dimensions
        .iter()
        .filter(|dimension| !own.contains(*dimension))
        .collect();
    if away.is_empty() {
        return groups;
    }
    let mut folded: Vec<GroupedUsageData> = Vec::with_capacity(groups.len());
    for mut group in groups {
        group.dimensions.retain(|key, _| !away.contains(&key));
        match folded
            .iter_mut()
            .find(|known| known.dimensions == group.dimensions)
        {
            Some(known) => known.value += group.value,
            None => folded.push(group),
        }
    }
    folded
}

/// What a window's usage, read as `groups` up to `until`, prices at, honouring `pinned` draft prices.
pub fn price_window_groups(
    window: &PlannedWindow,
    until: NaiveDateTime,
    groups: Vec<GroupedUsageData>,
    pinned: &PinnedPrices,
) -> StoreResult<i64> {
    let usage = convert_usage_units(
        &window.metric,
        UsageData {
            data: folded(window, groups),
            period: UsagePeriod {
                start: window.start,
                end: until,
            },
        },
    );
    let lines = match (&window.fee, &window.card) {
        (MeteredFee::Usage(_), Some(card)) => rated_usage_lines(
            &billing_core::pricing::UsageModel::RateCard(card.clone()),
            &usage.data,
            window.precision,
            |_| None,
        )?,
        (MeteredFee::Usage(model), None) => {
            usage_lines(model, &usage.data, window.precision, |dimensions| {
                pinned.unit_price(
                    window.metric.id,
                    window.sub_component_id,
                    window.sub_add_on_id,
                    dimensions,
                )
            })?
        }
        (
            MeteredFee::CapacityOverage {
                included,
                overage_rate,
            },
            _,
        ) => capacity_overage_lines(*included, overage_rate, &usage.data, window.precision)?,
    };
    lines.iter().try_fold(0_i64, |total, line| {
        total
            .checked_add(line.total)
            .ok_or_else(|| invalid("pending usage exceeds the supported range"))
    })
}
