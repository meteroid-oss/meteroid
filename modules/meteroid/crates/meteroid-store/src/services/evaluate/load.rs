//! A subscription's facts as `evaluate` reads them, from the rows that record them (Phase 7):
//! nothing is inferred from its state, and a fact still to be decided fails the load.

use std::collections::{BTreeMap, HashMap};

use billing_core::charge::Fee;
use billing_core::facts::{
    CouponFact, Ending, EndingKind, Facts, FeeFact, MinimumFact, MinimumScope, PlanWindow,
    SeatStep, Start, Trial,
};
use chrono::{Datelike, NaiveDate};
use common_domain::ids::{
    BaseId, BillableMetricId, CustomerId, SlotTransactionId, SubscriptionId, TenantId,
};
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::enums::SlotTransactionStatusEnum;
use diesel_models::slot_transactions::SlotTransactionRow;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_adjustments::{
    SubscriptionAdjustmentComponentRow, SubscriptionAdjustmentRow,
};
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscription_facts::{
    ENDING_CANCEL, ENDING_COMPLETE, SubscriptionEndingRow, SubscriptionPlanWindowRow,
    SubscriptionStartSettledRow, SubscriptionTrialRow,
};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use uuid::Uuid;

use super::pricer::{Resolved, StoreFee, fee_of, pinned_prices};
use crate::StoreResult;
use crate::constants::Currencies;
use crate::domain::enums::BillingPeriodEnum;
use crate::domain::{
    AppliedCouponDetailed, BillableMetric, LineItem, Subscription, SubscriptionAddOn,
    SubscriptionComponent, SubscriptionFee,
};
use crate::errors::StoreError;
use crate::repositories::subscriptions::{
    fetch_price_credit_units, fetch_prices_and_products, resolve_fee_from_maps,
    restore_stored_fee_credit_units, usage_attribution,
};
use crate::store::PgConn;

/// A fact the rows do not hold yet: the load fails rather than guess it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FactsError {
    #[error(
        "subscription {subscription}: the cadence origin of plan windows {windows:?} is still to be decided"
    )]
    OriginPending {
        subscription: SubscriptionId,
        windows: Vec<Uuid>,
    },
    #[error("subscription {subscription} has no plan window")]
    NoPlanWindow { subscription: SubscriptionId },
    #[error("subscription {subscription}: fee row {row} has no price to bill")]
    UnpricedFee {
        subscription: SubscriptionId,
        row: Uuid,
    },
}

fn facts_error(error: FactsError) -> Report<StoreError> {
    Report::new(error).change_context(StoreError::InvoiceComputationError)
}

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::<Report<StoreError>>::into)
}

/// What [`load_facts`] read: the facts, and what reading their usage needs.
#[derive(Clone, Debug)]
pub struct LoadedFacts {
    pub facts: Facts<StoreFee>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    /// The metrics the fees read, scoped as the subscription's usage is attributed (D2).
    pub metrics: Vec<BillableMetric>,
    /// Seats bought and still paying: not facts until their payment settles.
    pub pending_seats: Vec<SlotTransactionId>,
}

/// A fee row resolved to the domain, a closed one as stored, an open one from its price.
enum Row {
    Component(SubscriptionComponent),
    AddOn(SubscriptionAddOn),
}

impl Row {
    fn id(&self) -> Uuid {
        match self {
            Row::Component(c) => c.id.as_uuid(),
            Row::AddOn(a) => a.id.as_uuid(),
        }
    }

    fn lineage(&self) -> Fee {
        match self {
            Row::Component(c) => Fee::Component(c.lineage().as_uuid()),
            Row::AddOn(a) => Fee::AddOn(a.lineage().as_uuid()),
        }
    }

    fn window(&self) -> (NaiveDate, Option<NaiveDate>) {
        match self {
            Row::Component(c) => (c.effective_from, c.effective_to),
            Row::AddOn(a) => (a.effective_from, a.effective_to),
        }
    }

    fn fee(&self) -> &SubscriptionFee {
        match self {
            Row::Component(c) => &c.fee,
            Row::AddOn(a) => &a.fee,
        }
    }

    fn store_fee(&self, resolved: Resolved<'_>) -> StoreResult<StoreFee> {
        match self {
            Row::Component(c) => fee_of(c, resolved),
            Row::AddOn(a) => fee_of(a, resolved),
        }
    }

    fn pinned(
        &self,
        draft: Option<&[LineItem]>,
    ) -> BTreeMap<BTreeMap<String, String>, rust_decimal::Decimal> {
        match (self, draft) {
            (_, None) => BTreeMap::new(),
            (Row::Component(c), Some(draft)) => pinned_prices(c, draft),
            (Row::AddOn(a), Some(draft)) => pinned_prices(a, draft),
        }
    }
}

/// The facts of `subscription_id`, read on `conn` in the caller's snapshot. A row closed by
/// `as_of` is read as stored, an open one from its price, as invoices read them; `draft` is the
/// draft being refreshed, whose pinned unit prices its usage keeps.
pub async fn load_facts(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    as_of: NaiveDate,
    draft: Option<&[LineItem]>,
) -> StoreResult<LoadedFacts> {
    let subscription: Subscription =
        db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
            .try_into()?;
    let currency = Currencies::resolve_currency(&subscription.currency).ok_or_else(|| {
        Report::new(StoreError::ValueNotFound(format!(
            "Currency {} not found",
            subscription.currency
        )))
    })?;

    let plan_windows = plan_windows(conn, subscription_id).await?;
    let trial = db(SubscriptionTrialRow::find(conn, subscription_id).await)?.map(|row| Trial {
        from: row.from_date,
        until: row.until_date,
        free: row.free,
    });
    let settled = db(SubscriptionStartSettledRow::find(conn, subscription_id).await)?;
    let (ending, end_date) = endings(db(
        SubscriptionEndingRow::list_of(conn, subscription_id).await
    )?);

    let rows = fee_rows(conn, tenant_id, subscription_id, as_of).await?;
    let metrics = metrics_of(conn, &subscription, &rows).await?;
    let mut fees = Vec::with_capacity(rows.len());
    for row in &rows {
        let fee = row.store_fee(Resolved {
            currency: &currency,
            metrics: &metrics,
            pinned: row.pinned(draft),
        })?;
        let (from, to) = row.window();
        fees.push(FeeFact {
            id: row.id(),
            lineage: row.lineage(),
            from,
            to,
            billing: fee.billing(),
            cadence: fee.cadence(),
            fee,
        });
    }

    let transactions = db(SlotTransactionRow::list_by_subscription_id(
        conn,
        tenant_id,
        subscription_id,
        None,
        None,
    )
    .await)?;
    let seats = seat_steps(&rows, &transactions);
    let pending_seats = transactions
        .iter()
        .filter(|t| t.status == SlotTransactionStatusEnum::Pending)
        .map(|t| t.id)
        .collect();

    Ok(LoadedFacts {
        facts: Facts {
            subscription: subscription_id.as_uuid(),
            billing_day_anchor: u32::from(subscription.billing_day_anchor),
            start: Start {
                date: subscription.start_date,
                settled: settled.map(|row| row.at.and_utc()),
            },
            trial,
            plan_windows,
            fees,
            seats,
            coupons: coupons(conn, subscription_id).await?,
            minimums: minimums(conn, subscription_id).await?,
            ending,
            end_date,
            // The reconcile watermark is recorded from PR 3 on.
            reconcile_from: None,
        },
        tenant_id,
        customer_id: subscription.customer_id,
        metrics,
        pending_seats,
    })
}

async fn plan_windows(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
) -> StoreResult<Vec<PlanWindow>> {
    let pending = db(SubscriptionPlanWindowRow::pending_origins_of(conn, subscription_id).await)?;
    if !pending.is_empty() {
        return Err(facts_error(FactsError::OriginPending {
            subscription: subscription_id,
            windows: pending,
        }));
    }
    let windows: Vec<PlanWindow> =
        db(SubscriptionPlanWindowRow::list_by_subscription(conn, subscription_id).await)?
            .into_iter()
            .map(|row| PlanWindow {
                plan_version: row.plan_version_id.as_uuid(),
                from: row.from_date,
                to: row.to_date,
                cadence: BillingPeriodEnum::from(row.cadence).as_months(),
                cadence_origin: row.cadence_origin,
            })
            .collect();
    if windows.is_empty() {
        return Err(facts_error(FactsError::NoPlanWindow {
            subscription: subscription_id,
        }));
    }
    Ok(windows)
}

/// The cancellation as the ending, the fixed term as the end date: the core takes the earlier.
fn endings(rows: Vec<SubscriptionEndingRow>) -> (Option<Ending>, Option<NaiveDate>) {
    let mut ending = None;
    let mut end_date = None;
    for row in rows {
        match row.kind.as_str() {
            ENDING_CANCEL => {
                ending = Some(Ending {
                    at: row.at.and_utc(),
                    kind: EndingKind::Cancel,
                });
            }
            ENDING_COMPLETE => end_date = Some(row.at.date()),
            _ => {}
        }
    }
    (ending, end_date)
}

/// Every component and add-on row: a priced (v2) row from its price, a stored one as stored, its
/// credit unit restored once closed, as `load_closed_arrears` reads closed rows.
async fn fee_rows(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    as_of: NaiveDate,
) -> StoreResult<Vec<Row>> {
    let components = db(SubscriptionComponentRow::list_fact_rows(conn, subscription_id).await)?;
    let add_ons = db(SubscriptionAddOnRow::list_fact_rows(conn, subscription_id).await)?;
    let (prices, products) = fetch_prices_and_products(
        conn,
        tenant_id,
        components
            .iter()
            .filter_map(|r| r.price_id)
            .chain(add_ons.iter().filter_map(|r| r.price_id)),
        components
            .iter()
            .filter_map(|r| r.product_id)
            .chain(add_ons.iter().filter_map(|r| r.product_id)),
    )
    .await?;
    let units = fetch_price_credit_units(conn, tenant_id, &prices).await?;
    let unpriced = |row: Uuid| {
        facts_error(FactsError::UnpricedFee {
            subscription: subscription_id,
            row,
        })
    };

    let mut rows = Vec::with_capacity(components.len() + add_ons.len());
    let mut restore: Vec<usize> = Vec::new();
    for row in components {
        let closed = row.effective_to.is_some_and(|to| to <= as_of);
        let component = match row.price_id {
            Some(_) => {
                let (period, fee) =
                    resolve_fee_from_maps(row.price_id, row.product_id, &prices, &products, &units)
                        .ok_or_else(|| unpriced(row.id.as_uuid()))?;
                SubscriptionComponent {
                    id: row.id,
                    price_component_id: row.price_component_id,
                    product_id: row.product_id,
                    subscription_id: row.subscription_id,
                    name: row.name,
                    period,
                    fee,
                    price_id: row.price_id,
                    effective_from: row.effective_from,
                    effective_to: row.effective_to,
                    lineage_id: row.lineage_id,
                    added_by_amendment: row.added_by_amendment,
                }
            }
            None => {
                if closed {
                    restore.push(rows.len());
                }
                row.try_into()?
            }
        };
        rows.push(Row::Component(component));
    }
    for row in add_ons {
        let closed = row.effective_to.is_some_and(|to| to <= as_of);
        let add_on = match row.price_id {
            Some(_) => {
                let (period, fee) =
                    resolve_fee_from_maps(row.price_id, row.product_id, &prices, &products, &units)
                        .ok_or_else(|| unpriced(row.id.as_uuid()))?;
                SubscriptionAddOn {
                    id: row.id,
                    subscription_id: row.subscription_id,
                    add_on_id: row.add_on_id,
                    name: row.name,
                    period,
                    fee,
                    created_at: row.created_at,
                    product_id: row.product_id,
                    price_id: row.price_id,
                    quantity: row.quantity,
                    effective_from: row.effective_from,
                    effective_to: row.effective_to,
                    lineage_id: row.lineage_id,
                    added_by_amendment: row.added_by_amendment,
                }
            }
            None => {
                if closed {
                    restore.push(rows.len());
                }
                row.try_into()?
            }
        };
        rows.push(Row::AddOn(add_on));
    }
    restore_stored_fee_credit_units(
        conn,
        tenant_id,
        rows.iter_mut()
            .enumerate()
            .filter(|(i, _)| restore.contains(i))
            .map(|(_, row)| match row {
                Row::Component(c) => (c.price_id, &mut c.fee),
                Row::AddOn(a) => (a.price_id, &mut a.fee),
            })
            .collect(),
    )
    .await?;
    Ok(rows)
}

/// The metrics the rows read, scoped by the subscription's usage attribution (D2).
async fn metrics_of(
    conn: &mut PgConn,
    subscription: &Subscription,
    rows: &[Row],
) -> StoreResult<Vec<BillableMetric>> {
    let mut ids: Vec<BillableMetricId> = Vec::new();
    for id in rows.iter().filter_map(|row| row.fee().metric_id()) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let scopes = usage_attribution::subscription_usage_scopes(conn, subscription).await?;
    db(BillableMetricRow::get_by_ids(conn, &ids, &subscription.tenant_id).await)?
        .into_iter()
        .map(|row| TryInto::<BillableMetric>::try_into(row).map(|metric| scopes.apply(metric)))
        .collect()
}

/// The seats each slot fee bills from the day an active transaction changed them, the day's last
/// one counting. Pending upgrades are no fact until their payment settles.
fn seat_steps(rows: &[Row], transactions: &[SlotTransactionRow]) -> Vec<SeatStep> {
    let mut lineages: Vec<(&str, Fee)> = rows
        .iter()
        .filter_map(|row| match row.fee() {
            SubscriptionFee::Slot { unit, .. } => Some((unit.as_str(), row.lineage())),
            _ => None,
        })
        .collect();
    lineages.sort();
    lineages.dedup();
    let mut active: Vec<&SlotTransactionRow> = transactions
        .iter()
        .filter(|t| t.status == SlotTransactionStatusEnum::Active)
        .collect();
    active.sort_by_key(|t| (t.effective_at, t.transaction_at));
    let mut by_day: BTreeMap<(Fee, NaiveDate), SeatStep> = BTreeMap::new();
    for transaction in active {
        let quantity =
            u64::try_from(transaction.prev_active_slots + transaction.delta).unwrap_or(0);
        for (_, lineage) in lineages
            .iter()
            .filter(|(unit, _)| *unit == transaction.unit)
        {
            let at = transaction.effective_at.date();
            by_day.insert(
                (lineage.clone(), at),
                SeatStep {
                    id: transaction.id.as_uuid(),
                    lineage: lineage.clone(),
                    at,
                    quantity,
                },
            );
        }
    }
    by_day.into_values().collect()
}

/// The coupons discounting invoices: from their date, for their invoices; a credit coupon pays
/// through the wallet and a removed one discounts nothing.
async fn coupons(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
) -> StoreResult<Vec<CouponFact>> {
    let rows = db(
        diesel_models::applied_coupons::AppliedCouponDetailedRow::list_by_subscription_id(
            conn,
            &subscription_id,
        )
        .await,
    )?;
    let mut coupons = Vec::new();
    for row in rows {
        let from = row.applied_coupon.effective_from;
        let applied: AppliedCouponDetailed = row.try_into()?;
        if applied.coupon.discount.is_credit() || !applied.applied_coupon.is_active {
            continue;
        }
        coupons.push(CouponFact {
            id: applied.applied_coupon.id.as_uuid(),
            from,
            invoices: applied
                .coupon
                .recurring_value
                .map(|n| u32::try_from(n).unwrap_or(0)),
        });
    }
    Ok(coupons)
}

/// The dated minimums. The core names one product per scope: a minimum over several products is
/// read under its first, its commitment never compared before PR 3 (R-022, OQ17).
async fn minimums(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
) -> StoreResult<Vec<MinimumFact>> {
    let rows = db(SubscriptionAdjustmentRow::list_minimum_facts(conn, subscription_id).await)?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut products: HashMap<NaiveDate, Vec<Uuid>> = HashMap::new();
    for scope in db(
        SubscriptionAdjustmentComponentRow::list_minimum_fact_scopes(conn, subscription_id).await,
    )? {
        products
            .entry(scope.effective_from)
            .or_default()
            .push(scope.product_id.as_uuid());
    }
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let scope = if row.includes_all_components {
                MinimumScope::AllComponents
            } else {
                MinimumScope::Product(products.get(&row.effective_from)?.iter().min().copied()?)
            };
            Some(MinimumFact {
                // A minimum is keyed by its date: one per kind starts on a day.
                id: Uuid::from_u128(u128::from(
                    row.effective_from.num_days_from_ce().unsigned_abs(),
                )),
                from: row.effective_from,
                to: row.effective_to,
                amount: row.amount_subunit,
                scope,
            })
        })
        .collect())
}
