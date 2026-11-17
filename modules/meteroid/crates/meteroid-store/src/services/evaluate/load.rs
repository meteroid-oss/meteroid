//! A subscription's facts as `evaluate` reads them, from the rows that record them:
//! nothing is inferred from its state, and a fact still to be decided fails the load.

use std::collections::{BTreeMap, HashMap};

use billing_core::charge::Fee;
use billing_core::facts::{
    CouponFact, Ending, EndingKind, Facts, FeeFact, MinimumFact, MinimumScope, PlanWindow,
    SeatStep, Start, Trial,
};
use chrono::{Datelike, NaiveDate};
use common_domain::ids::{
    BaseId, BillableMetricId, CustomerId, PriceId, ProductId, SlotTransactionId, SubscriptionId,
    TenantId,
};
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::enums::SlotTransactionStatusEnum;
use diesel_models::prices::PriceRow;
use diesel_models::slot_transactions::SlotTransactionRow;
use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use diesel_models::subscription_adjustments::{
    SubscriptionAdjustmentComponentRow, SubscriptionAdjustmentRow,
};
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
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
use crate::domain::Price;
use crate::domain::enums::BillingPeriodEnum;
use crate::domain::{
    AppliedCouponDetailed, BillableMetric, LineItem, Subscription, SubscriptionAddOn,
    SubscriptionComponent, SubscriptionDetails, SubscriptionFee,
};
use crate::errors::StoreError;
use crate::repositories::subscriptions::{
    fetch_price_credit_units, fetch_price_lineages, fetch_prices_and_products, price_in_force,
    resolve_fee_from_maps, restore_stored_fee_credit_units, usage_attribution,
};
use crate::services::facts::pending::{MinimumChange, PendingFacts, PlanWindowFact, SeatSeed};
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
            Row::Component(c) => Fee::Component(c.lineage()),
            Row::AddOn(a) => Fee::AddOn(a.lineage()),
        }
    }

    /// A component of the plan version it was written with, not one an amendment added.
    fn plan_component(&self) -> bool {
        matches!(self, Row::Component(c) if !c.added_by_amendment && c.price_component_id.is_some())
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
        let Some(draft) = draft else {
            return BTreeMap::new();
        };
        // A row cut by a dated price pins only the lines of its own part (R-097).
        let (from, to) = self.window();
        let own: Vec<LineItem> = draft
            .iter()
            .filter(|line| line.start_date >= from && to.is_none_or(|to| line.end_date <= to))
            .cloned()
            .collect();
        match self {
            Row::Component(c) => pinned_prices(c, &own),
            Row::AddOn(a) => pinned_prices(a, &own),
        }
    }
}

/// A priced row's window `[from, to)` cut at each successor of its price's lineage taking over
/// inside it, with the lineage member in force on each part (R-097).
fn price_parts(
    (from, to): (NaiveDate, Option<NaiveDate>),
    lineage: &[PriceRow],
) -> Vec<(NaiveDate, Option<NaiveDate>, PriceId)> {
    let mut cuts: Vec<NaiveDate> = lineage
        .iter()
        .filter_map(|row| row.effective_from)
        .filter(|at| *at > from && to.is_none_or(|to| *at < to))
        .collect();
    cuts.sort();
    cuts.dedup();
    let starts: Vec<NaiveDate> = std::iter::once(from).chain(cuts.iter().copied()).collect();
    starts
        .iter()
        .enumerate()
        .filter_map(|(i, start)| {
            let member = price_in_force(lineage, *start)?;
            Some((*start, starts.get(i + 1).copied().or(to), member.id))
        })
        .collect()
}

/// The rows a change not yet written would write, laid over the stored ones by a preview, which
/// writes nothing.
#[derive(Clone, Debug)]
pub struct Overlay {
    pub rows: PendingFacts,
    /// The billing day a change ending a free trial moves the grid to.
    pub billing_day_anchor: Option<u32>,
}

impl Overlay {
    pub fn of(rows: PendingFacts) -> Self {
        Self {
            rows,
            billing_day_anchor: None,
        }
    }
}

/// The facts of `subscription_id`, read on `conn` in the caller's snapshot. A legacy row is read
/// as stored, a priced row from the price in force on each day (R-097); `draft` is the draft
/// being refreshed, whose pinned unit prices its usage keeps.
pub async fn load_facts(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    as_of: NaiveDate,
    draft: Option<&[LineItem]>,
) -> StoreResult<LoadedFacts> {
    load_facts_with(conn, tenant_id, subscription_id, as_of, draft, None).await
}

/// [`load_facts`], with the rows of `overlay` laid over the stored ones as writing them would.
pub async fn load_facts_with(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    as_of: NaiveDate,
    draft: Option<&[LineItem]>,
    overlay: Option<&Overlay>,
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

    let mut plan_windows = plan_windows(conn, subscription_id).await?;
    let mut trial = db(SubscriptionTrialRow::find(conn, subscription_id).await)?.map(|row| Trial {
        from: row.from_date,
        until: row.until_date,
        free: row.free,
    });
    let settled = db(SubscriptionStartSettledRow::find(conn, subscription_id).await)?;
    let (ending, end_date) = endings(db(
        SubscriptionEndingRow::list_of(conn, subscription_id).await
    )?);
    let change = overlay.map(|overlay| &overlay.rows);
    if let Some(window) = change.and_then(|change| change.plan_window.as_ref()) {
        lay_plan_window(&mut plan_windows, window);
        if let (Some(trial), Some(until)) = (trial.as_mut(), window.trial_until) {
            trial.until = until;
        }
    }

    let rows = fee_rows(conn, tenant_id, subscription_id, as_of, change).await?;
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
            plan_component: row.plan_component(),
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
    let mut seats = seat_steps(&rows, &transactions);
    let pending_seats = transactions
        .iter()
        .filter(|t| t.status == SlotTransactionStatusEnum::Pending)
        .map(|t| t.id)
        .collect();
    let mut minimums = minimums(conn, subscription_id).await?;
    if let Some(change) = change {
        lay_seat_seeds(&mut seats, &rows, &change.seat_seeds, change.on);
        lay_seat_seeds(&mut seats, &rows, &change.set_seats, change.on);
        lay_minimum(&mut minimums, &change.minimum, change.on);
    }

    Ok(LoadedFacts {
        facts: Facts {
            subscription: subscription_id,
            billing_day_anchor: overlay
                .and_then(|overlay| overlay.billing_day_anchor)
                .unwrap_or(u32::from(subscription.billing_day_anchor)),
            start: Start {
                date: subscription.start_date,
                settled: settled.map(|row| row.at.and_utc()),
            },
            trial,
            plan_windows,
            fees,
            seats,
            coupons: coupons(conn, subscription_id).await?,
            minimums,
            ending,
            end_date,
            reconcile_from: db(SubscriptionRow::reconcile_from(conn, subscription_id).await)?,
        },
        tenant_id,
        customer_id: subscription.customer_id,
        metrics,
        pending_seats,
    })
}

/// An add-on row as the facts read it, before it is written: from its stored fee, so a purchase is
/// priced the same whether its row is written yet or not.
pub fn add_on_fact(
    add_on: &SubscriptionAddOn,
    currency: &str,
    metrics: &[BillableMetric],
) -> StoreResult<FeeFact<StoreFee>> {
    let currency = Currencies::resolve_currency(currency).ok_or_else(|| {
        Report::new(StoreError::ValueNotFound(format!(
            "Currency {currency} not found"
        )))
    })?;
    let row = Row::AddOn(add_on.clone());
    let fee = row.store_fee(Resolved {
        currency: &currency,
        metrics,
        pinned: BTreeMap::new(),
    })?;
    let (from, to) = row.window();
    Ok(FeeFact {
        id: row.id(),
        lineage: row.lineage(),
        from,
        to,
        billing: fee.billing(),
        cadence: fee.cadence(),
        plan_component: row.plan_component(),
        fee,
    })
}

/// The facts a subscription not created yet would record from `details`: one plan window from its
/// billing start, its rows and the seats its slot fees start with. Without its trial: a preview
/// prices what it bills from its billing start.
pub(crate) async fn facts_of_details(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
) -> StoreResult<LoadedFacts> {
    let subscription = &details.subscription;
    let currency = Currencies::resolve_currency(&subscription.currency).ok_or_else(|| {
        Report::new(StoreError::ValueNotFound(format!(
            "Currency {} not found",
            subscription.currency
        )))
    })?;
    let start = subscription
        .billing_start_date
        .unwrap_or(subscription.start_date);
    let rows: Vec<Row> = details
        .price_components
        .iter()
        .cloned()
        .map(Row::Component)
        .chain(details.add_ons.iter().cloned().map(Row::AddOn))
        .collect();
    let metrics = metrics_of(conn, subscription, &rows).await?;
    let mut fees = Vec::with_capacity(rows.len());
    for row in &rows {
        let fee = row.store_fee(Resolved {
            currency: &currency,
            metrics: &metrics,
            pinned: BTreeMap::new(),
        })?;
        let (from, to) = row.window();
        fees.push(FeeFact {
            id: row.id(),
            lineage: row.lineage(),
            from,
            to,
            billing: fee.billing(),
            cadence: fee.cadence(),
            plan_component: row.plan_component(),
            fee,
        });
    }
    let seats = rows
        .iter()
        .filter_map(|row| match row.fee() {
            SubscriptionFee::Slot { initial_slots, .. } => Some(SeatStep {
                id: SlotTransactionId::new(),
                lineage: row.lineage(),
                at: row.window().0,
                quantity: u64::from(*initial_slots),
            }),
            _ => None,
        })
        .collect();
    Ok(LoadedFacts {
        facts: Facts {
            subscription: subscription.id,
            billing_day_anchor: u32::from(subscription.billing_day_anchor),
            start: Start {
                date: start,
                settled: Some(super::bill::midnight(start)),
            },
            trial: None,
            plan_windows: vec![PlanWindow {
                plan_version: subscription.plan_version_id,
                from: start,
                to: None,
                cadence: subscription.period.as_months(),
                cadence_origin: start,
            }],
            fees,
            seats,
            coupons: Vec::new(),
            minimums: Vec::new(),
            ending: None,
            end_date: subscription.end_date,
            reconcile_from: None,
        },
        tenant_id: subscription.tenant_id,
        customer_id: subscription.customer_id,
        metrics,
        pending_seats: Vec::new(),
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
                plan_version: row.plan_version_id,
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

/// A plan window opened as `SubscriptionPlanWindowRow::open` opens it: the open one closed at its
/// start, or replaced when it opened on or after it.
fn lay_plan_window(windows: &mut Vec<PlanWindow>, window: &PlanWindowFact) {
    let mut from = window.from_date;
    if let Some(open) = windows.iter().rposition(|w| w.to.is_none()) {
        if windows[open].from < from {
            windows[open].to = Some(from);
        } else {
            from = windows[open].from;
            windows.remove(open);
        }
    }
    windows.push(PlanWindow {
        plan_version: window.plan_version_id,
        from,
        to: None,
        cadence: window.cadence.as_months(),
        cadence_origin: window.cadence_origin,
    });
}

/// The rows a change closes end on its date, those it opens start then.
fn lay_fee_rows(
    components: &mut Vec<SubscriptionComponentRow>,
    add_ons: &mut Vec<SubscriptionAddOnRow>,
    change: &PendingFacts,
) {
    let on = change.on;
    for row in components
        .iter_mut()
        .filter(|row| row.effective_to.is_none() && change.closes_components.contains(&row.id))
    {
        row.effective_to = Some(on);
    }
    for row in add_ons
        .iter_mut()
        .filter(|row| row.effective_to.is_none() && change.closes_add_ons.contains(&row.id))
    {
        row.effective_to = Some(on);
    }
    components.extend(change.opens_components.iter().map(component_row_of));
    add_ons.extend(change.opens_add_ons.iter().map(add_on_row_of));
}

/// A component row not written yet, as it reads once written.
pub(crate) fn component_row_of(row: &SubscriptionComponentRowNew) -> SubscriptionComponentRow {
    SubscriptionComponentRow {
        id: row.id,
        name: row.name.clone(),
        subscription_id: row.subscription_id,
        price_component_id: row.price_component_id,
        product_id: row.product_id,
        period: row.period.clone(),
        legacy_fee: row.legacy_fee.clone(),
        price_id: row.price_id,
        effective_from: row.effective_from,
        effective_to: None,
        lineage_id: row.lineage_id,
        added_by_amendment: row.added_by_amendment,
    }
}

/// An add-on row not written yet, as it reads once written.
pub(crate) fn add_on_row_of(row: &SubscriptionAddOnRowNew) -> SubscriptionAddOnRow {
    SubscriptionAddOnRow {
        id: row.id,
        name: row.name.clone(),
        subscription_id: row.subscription_id,
        add_on_id: row.add_on_id,
        period: row.period.clone(),
        legacy_fee: row.legacy_fee.clone(),
        created_at: common_clock::now().naive_utc(),
        product_id: row.product_id,
        price_id: row.price_id,
        quantity: row.quantity,
        effective_from: row.effective_from,
        effective_to: None,
        lineage_id: row.lineage_id,
        added_by_amendment: row.added_by_amendment,
    }
}

/// The seats a change records on `on` for each slot fee of their unit, as the slot transactions
/// it writes would read: the day's last step counts.
fn lay_seat_seeds(seats: &mut Vec<SeatStep>, rows: &[Row], seeds: &[SeatSeed], on: NaiveDate) {
    for seed in seeds {
        let mut lineages: Vec<Fee> = rows
            .iter()
            .filter(
                |row| matches!(row.fee(), SubscriptionFee::Slot { unit, .. } if *unit == seed.unit),
            )
            .map(Row::lineage)
            .collect();
        lineages.sort();
        lineages.dedup();
        for lineage in lineages {
            seats.retain(|step| !(step.lineage == lineage && step.at == on));
            seats.push(SeatStep {
                id: SlotTransactionId::new(),
                lineage,
                at: on,
                quantity: u64::try_from(seed.slots).unwrap_or(0),
            });
        }
    }
    seats.sort_by(|a, b| (&a.lineage, a.at).cmp(&(&b.lineage, b.at)));
}

/// What a plan change makes of the minimum from `on`, as `refresh_subscription_minimum_on_plan_change`
/// writes it.
fn lay_minimum(minimums: &mut Vec<MinimumFact>, change: &MinimumChange, on: NaiveDate) {
    let (amount, scope) = match change {
        MinimumChange::Kept => return,
        MinimumChange::Ended => (None, None),
        MinimumChange::Replaced {
            amount_subunit,
            includes_all_components,
            products,
        } => {
            let scope = if *includes_all_components {
                Some(MinimumScope::AllComponents)
            } else {
                products.iter().min().copied().map(MinimumScope::Product)
            };
            (Some(*amount_subunit), scope)
        }
    };
    minimums.retain(|minimum| minimum.from < on);
    for minimum in minimums.iter_mut() {
        if minimum.to.is_none_or(|to| to > on) {
            minimum.to = Some(on);
        }
    }
    if let (Some(amount), Some(scope)) = (amount, scope) {
        minimums.push(MinimumFact {
            id: Uuid::from_u128(u128::from(on.num_days_from_ce().unsigned_abs())),
            from: on,
            to: None,
            amount,
            scope,
        });
    }
}

/// Every component and add-on row: a priced (v2) row from its price, a stored one as stored, its
/// credit unit restored once closed, as `load_closed_arrears` reads closed rows.
async fn fee_rows(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    as_of: NaiveDate,
    change: Option<&PendingFacts>,
) -> StoreResult<Vec<Row>> {
    let mut components = db(SubscriptionComponentRow::list_fact_rows(conn, subscription_id).await)?;
    let mut add_ons = db(SubscriptionAddOnRow::list_fact_rows(conn, subscription_id).await)?;
    if let Some(change) = change {
        lay_fee_rows(&mut components, &mut add_ons, change);
    }
    let roots: Vec<PriceId> = components
        .iter()
        .filter_map(|r| r.price_id)
        .chain(add_ons.iter().filter_map(|r| r.price_id))
        .collect();
    let lineages = fetch_price_lineages(conn, tenant_id, &roots).await?;
    let (_, products) = fetch_prices_and_products(
        conn,
        tenant_id,
        std::iter::empty(),
        components
            .iter()
            .filter_map(|r| r.product_id)
            .chain(add_ons.iter().filter_map(|r| r.product_id)),
    )
    .await?;
    // Every member of each lineage, by its own id: a part is priced from the one in force.
    let mut prices: HashMap<PriceId, Price> = HashMap::new();
    for row in lineages.values().flatten() {
        prices.insert(row.id, Price::try_from(row.clone())?);
    }
    let units = fetch_price_credit_units(conn, tenant_id, &prices).await?;
    let parts = |price_id: Option<PriceId>, window: (NaiveDate, Option<NaiveDate>)| {
        price_id
            .and_then(|root| lineages.get(&root))
            .map(|lineage| price_parts(window, lineage))
            .unwrap_or_default()
    };
    let unpriced = |row: Uuid| {
        facts_error(FactsError::UnpricedFee {
            subscription: subscription_id,
            row,
        })
    };

    let mut rows = Vec::with_capacity(components.len() + add_ons.len());
    let mut restore: Vec<usize> = Vec::new();
    for row in components {
        if row.price_id.is_none() {
            if row.effective_to.is_some_and(|to| to <= as_of) {
                restore.push(rows.len());
            }
            rows.push(Row::Component(row.try_into()?));
            continue;
        }
        let window = (row.effective_from, row.effective_to);
        let parts = parts(row.price_id, window);
        if parts.is_empty() {
            return Err(unpriced(row.id.as_uuid()));
        }
        for (from, to, member) in parts {
            let (period, fee) =
                resolve_fee_from_maps(Some(member), row.product_id, &prices, &products, &units)
                    .ok_or_else(|| unpriced(row.id.as_uuid()))?;
            rows.push(Row::Component(SubscriptionComponent {
                id: row.id,
                price_component_id: row.price_component_id,
                product_id: row.product_id,
                subscription_id: row.subscription_id,
                name: row.name.clone(),
                period,
                fee,
                price_id: row.price_id,
                effective_from: from,
                effective_to: to,
                lineage_id: row.lineage_id,
                added_by_amendment: row.added_by_amendment,
            }));
        }
    }
    for row in add_ons {
        if row.price_id.is_none() {
            if row.effective_to.is_some_and(|to| to <= as_of) {
                restore.push(rows.len());
            }
            rows.push(Row::AddOn(row.try_into()?));
            continue;
        }
        let window = (row.effective_from, row.effective_to);
        let parts = parts(row.price_id, window);
        if parts.is_empty() {
            return Err(unpriced(row.id.as_uuid()));
        }
        for (from, to, member) in parts {
            let (period, fee) =
                resolve_fee_from_maps(Some(member), row.product_id, &prices, &products, &units)
                    .ok_or_else(|| unpriced(row.id.as_uuid()))?;
            rows.push(Row::AddOn(SubscriptionAddOn {
                id: row.id,
                subscription_id: row.subscription_id,
                add_on_id: row.add_on_id,
                name: row.name.clone(),
                period,
                fee,
                created_at: row.created_at,
                product_id: row.product_id,
                price_id: row.price_id,
                quantity: row.quantity,
                effective_from: from,
                effective_to: to,
                lineage_id: row.lineage_id,
                added_by_amendment: row.added_by_amendment,
            }));
        }
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
/// counting: one of no seats sets the count (first at its instant), the others add theirs, as the
/// ledger's reads do. Pending upgrades are no fact until their payment settles.
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
    active.sort_by_key(|t| (t.effective_at, t.delta != 0, t.transaction_at));
    let mut counts: BTreeMap<&str, i32> = BTreeMap::new();
    let mut by_day: BTreeMap<(Fee, NaiveDate), SeatStep> = BTreeMap::new();
    for transaction in active {
        let count = match counts.get(transaction.unit.as_str()) {
            Some(count) if transaction.delta != 0 => count + transaction.delta,
            _ => transaction.prev_active_slots + transaction.delta,
        };
        counts.insert(transaction.unit.as_str(), count);
        let quantity = u64::try_from(count).unwrap_or(0);
        for (_, lineage) in lineages
            .iter()
            .filter(|(unit, _)| *unit == transaction.unit)
        {
            let at = transaction.effective_at.date();
            by_day.insert(
                (lineage.clone(), at),
                SeatStep {
                    id: transaction.id,
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
            id: applied.applied_coupon.id,
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
/// read under its first, its commitment never compared before billing read the facts (R-022,
/// OQ17).
async fn minimums(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
) -> StoreResult<Vec<MinimumFact>> {
    let rows = db(SubscriptionAdjustmentRow::list_minimum_facts(conn, subscription_id).await)?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut products: HashMap<NaiveDate, Vec<ProductId>> = HashMap::new();
    for scope in db(
        SubscriptionAdjustmentComponentRow::list_minimum_fact_scopes(conn, subscription_id).await,
    )? {
        products
            .entry(scope.effective_from)
            .or_default()
            .push(scope.product_id);
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
