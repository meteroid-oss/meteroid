//! The usage windows an invoice bills and what their usage prices at, as invoicing derives them:
//! the upcoming invoice of each subscription, a draft's or a final invoice's own windows.
use crate::StoreResult;
use crate::constants::Currencies;
use crate::domain::{
    BillableMetric, Period, SubscriptionDetails, SubscriptionFeeInterface, UsagePeriod,
};
use crate::errors::StoreError;
use crate::repositories::credit::customer_credit::normalized_currency;
use crate::repositories::subscriptions::SubscriptionInterface;
use crate::services::clients::usage::{GroupedUsageData, UsageData};
use crate::services::credit::invalid;
use crate::services::credit::unit_wallets::OverageRates;
use crate::services::invoice_lines::component::{
    PinnedPrices, capacity_overage_lines, convert_usage_units, usage_lines, usage_window,
};
use crate::services::invoice_lines::invoice_lines::{
    BilledSpan, MeteredFee, MeteredWindow, details_for_span, load_closed_arrears, metered_windows,
};
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
    if details.subscription.status == crate::domain::SubscriptionStatusEnum::TrialActive {
        let row = diesel_models::subscriptions::SubscriptionRow::get_subscription_by_id(
            conn,
            &details.subscription.tenant_id,
            details.subscription.id,
        )
        .await?;
        if row.subscription.next_cycle_action
            == Some(diesel_models::enums::CycleActionEnum::EndTrial)
        {
            return Ok(Vec::new());
        }
    }
    let (upcoming, invoice_date) = upcoming_invoice_details(details);
    let cycle_index = upcoming.subscription.cycle_index.unwrap_or(0);
    windows_at(
        conn,
        &upcoming,
        invoice_date,
        cycle_index,
        BilledSpan::Period,
    )
    .await
}

/// The windows the invoice of `cycle_index`, dated `invoice_date`, bills for the subscription:
/// its period's own invoice, or its final one (`span`).
pub(crate) async fn windows_at(
    conn: &mut PgConn,
    upcoming: &SubscriptionDetails,
    invoice_date: NaiveDate,
    cycle_index: u32,
    span: BilledSpan,
) -> StoreResult<Vec<PlannedWindow>> {
    // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
    Box::pin(windows_at_unboxed(
        conn,
        upcoming,
        invoice_date,
        cycle_index,
        span,
    ))
    .await
}

async fn windows_at_unboxed(
    conn: &mut PgConn,
    upcoming: &SubscriptionDetails,
    invoice_date: NaiveDate,
    cycle_index: u32,
    span: BilledSpan,
) -> StoreResult<Vec<PlannedWindow>> {
    let reading = details_for_span(conn, upcoming, span).await?;
    let upcoming: &SubscriptionDetails = reading.as_deref().unwrap_or(upcoming);
    let Some(billing_start_date) = upcoming.subscription.billing_start_date else {
        return Ok(Vec::new());
    };

    let currency = normalized_currency(&upcoming.subscription.currency);
    let precision = Currencies::resolve_currency(&currency)
        .ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(format!(
                "Currency {currency} not found"
            )))
        })?
        .precision;

    let closed = load_closed_arrears(conn, upcoming, invoice_date).await?;
    let closed_details = closed.details_for(upcoming);
    let runs = crate::services::invoice_lines::cadence::load_cadence_runs(
        conn,
        upcoming,
        invoice_date,
        true,
    )
    .await?;

    let mut planned = Vec::new();
    // The credit-priced windows, by index in `planned`, whose overage rate is resolved below.
    let mut credit_priced: Vec<(usize, CreditUnitId, String, PlanVersionId)> = Vec::new();
    let mut plan_windows = |details: &SubscriptionDetails, windows: Vec<MeteredWindow>| {
        for window in windows {
            let metric = details
                .metrics
                .iter()
                .find(|metric| metric.id == window.metric_id)
                .ok_or_else(|| {
                    Report::new(StoreError::ValueNotFound(format!(
                        "metric with id {}",
                        window.metric_id
                    )))
                })?;
            let usage = usage_window(details, window.period.clone());
            if usage.end <= usage.start {
                continue;
            }
            // Usage priced in a credit unit is owed to that unit's wallet, in its minor units;
            // what it leaves uncovered is owed in money.
            if let Some(unit) = &window.credit_unit {
                credit_priced.push((
                    planned.len(),
                    unit.unit_id,
                    unit.code.clone(),
                    details.subscription.plan_version_id,
                ));
            }
            let (overage, (currency, precision)) = match &window.credit_unit {
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
            let product_id = details
                .price_components
                .iter()
                .find(|component| {
                    window.sub_component_id.is_some()
                        && component.sub_component_id() == window.sub_component_id
                })
                .and_then(|component| component.product_id())
                .or_else(|| {
                    details
                        .add_ons
                        .iter()
                        .find(|add_on| {
                            window.sub_add_on_id.is_some()
                                && add_on.sub_add_on_id() == window.sub_add_on_id
                        })
                        .and_then(|add_on| add_on.product_id())
                });
            planned.push(PlannedWindow {
                subscription_id: details.subscription.id,
                metric: metric.clone(),
                fee: window.fee,
                sub_component_id: window.sub_component_id,
                sub_add_on_id: window.sub_add_on_id,
                product_id,
                tax_inclusive: details.subscription.tax_inclusive,
                taxes: Vec::new(),
                period: window.period,
                start: usage.start,
                end: usage.end,
                currency,
                precision,
                overage,
            });
        }
        Ok::<_, Report<StoreError>>(())
    };

    plan_windows(
        upcoming,
        metered_windows(
            upcoming,
            &upcoming.price_components,
            invoice_date,
            billing_start_date,
            cycle_index,
            &runs,
            span,
        ),
    )?;
    plan_windows(
        upcoming,
        metered_windows(
            upcoming,
            &upcoming.add_ons,
            invoice_date,
            billing_start_date,
            cycle_index,
            &runs,
            span,
        ),
    )?;
    plan_windows(
        &closed_details,
        metered_windows(
            &closed_details,
            &closed.components,
            invoice_date,
            billing_start_date,
            cycle_index,
            &runs,
            span,
        ),
    )?;
    plan_windows(
        &closed_details,
        metered_windows(
            &closed_details,
            &closed.add_ons,
            invoice_date,
            billing_start_date,
            cycle_index,
            &runs,
            span,
        ),
    )?;
    if !credit_priced.is_empty() {
        let subscription = &upcoming.subscription;
        let unit_ids: Vec<CreditUnitId> = credit_priced.iter().map(|(_, id, ..)| *id).collect();
        let plan_version_ids: Vec<PlanVersionId> =
            credit_priced.iter().map(|(.., version)| *version).collect();
        let mut rates = OverageRates::load(
            conn,
            subscription.tenant_id,
            subscription.customer_id,
            &currency,
            &unit_ids,
            &plan_version_ids,
        )
        .await?;
        for (index, unit_id, code, plan_version_id) in credit_priced {
            if let Some(overage) = planned[index].overage.as_mut() {
                overage.rate = rates.rate(unit_id, &code, plan_version_id, overage.rate);
            }
        }
    }
    Ok(planned)
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
            data: groups,
            period: UsagePeriod {
                start: window.start,
                end: until,
            },
        },
    );
    let lines = match &window.fee {
        MeteredFee::Usage(model) => usage_lines(
            model,
            window.metric.id,
            &usage.data,
            &window.period,
            window.precision,
            |dimensions| {
                pinned.unit_price(
                    window.metric.id,
                    window.sub_component_id,
                    window.sub_add_on_id,
                    dimensions,
                )
            },
        )?,
        MeteredFee::CapacityOverage {
            included,
            overage_rate,
        } => capacity_overage_lines(
            window.metric.id,
            *included,
            overage_rate,
            &usage.data,
            &window.period,
            window.precision,
        )?,
    };
    lines.iter().try_fold(0_i64, |total, line| {
        i64::try_from(line.total)
            .ok()
            .and_then(|amount| total.checked_add(amount))
            .ok_or_else(|| invalid("pending usage exceeds the supported range"))
    })
}
