//! What an immediate plan change settles of what the old plan billed in advance: the unused part
//! of its fees, credited on notes against the invoices that billed them (their tax
//! reversed as it was charged), and the credit of its packs tied to the period, removed once it
//! paid the usage up to the change, the fee credited no higher than the unused paid credit.
//!
//! An invoice of the period still a draft at the change is settled the same way once issued,
//! from what the change recorded: the components it closed and its plan switch.
use crate::StoreResult;
use crate::domain::credit_notes::CreditNote;
use crate::domain::credit_units::CreditUnitRef;
use crate::domain::entity_activity::Actor;
use crate::domain::enums::SubscriptionFeeBillingPeriod;
use crate::domain::invoice_lines::LineItem;
use crate::domain::subscription_changes::{PackChange, PackChangeKind};
use crate::domain::subscription_changes::{ProrationLineItem, ProrationResult};
use crate::domain::{
    Invoice, LineItemKind, Subscription, SubscriptionAddOn, SubscriptionComponent,
    SubscriptionDetails, SubscriptionFee,
};
use crate::errors::StoreError;
use crate::repositories::credit::customer_credit::{
    self as credit, PeriodCreditRemoval, lock_customer_tx, lock_grants_tx, normalized_currency,
    source,
};
use crate::repositories::credit_notes::CreditLineItem;
use crate::services::Services;
use crate::services::credit::expiry_settlement::{read_ending_usage, settle_ending_grants_tx};
use crate::services::invoice_lines::minimum;
use crate::services::served_usage::UsageReads;
use crate::services::subscriptions::amendment::{
    PeriodCreditContext, gather_period_credit_context,
};
use crate::services::subscriptions::plan_change::PreparedPlanChange;
use crate::services::subscriptions::proration::{
    DayFraction, capped_pack_fee_credit, component_advance_amount_cents, is_period_tied,
};
use crate::services::subscriptions::utils::scale_fee;
use crate::store::PgConn;
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{
    BaseId, CreditGrantId, CreditNoteId, CustomerId, InvoiceId, PriceComponentId,
    SubscriptionAddOnId, SubscriptionId, TenantId,
};
use diesel_models::customer_credit::CustomerCreditGrantRow;
use diesel_models::lock::CustomerLock;
use diesel_models::slot_transactions::SlotTransactionRow;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_events::SettlingEnding;
use error_stack::Report;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use std::collections::{BTreeMap, HashMap, HashSet};

pub(crate) use crate::domain::credit::{
    CANCELLATION_NOTE_REASON, LATE_PAYMENT_NOTE_REASON, PLAN_CHANGE_NOTE_REASON,
};

/// The credit an old pack line funded for the period the change ends early.
#[derive(Debug, Clone)]
pub(crate) struct OutgoingPack {
    pub name: String,
    pub invoice_id: InvoiceId,
    pub line_local_id: String,
    pub unit: Option<CreditUnitRef>,
    /// Decimal places of the credit: the unit's, or the invoice currency's.
    pub precision: u8,
    /// Subtotal of the pack line credited back.
    pub fee_credit: i64,
    /// The line's grants, as the usage up to the change left them.
    pub grants: Vec<CustomerCreditGrantRow>,
}

impl OutgoingPack {
    pub(crate) fn changes(&self) -> Vec<PackChange> {
        let mut changes: Vec<PackChange> = self
            .grants
            .iter()
            .filter(|grant| grant.remaining > 0)
            .map(|grant| PackChange {
                name: self.name.clone(),
                kind: PackChangeKind::CreditRemoved,
                amount_cents: 0,
                credit_minor: grant.remaining,
                credit_unit: self.unit.clone(),
                credit_precision: self.precision,
                promotional: grant.classification != PAID,
            })
            .collect();
        if self.fee_credit > 0 {
            changes.push(PackChange {
                name: self.name.clone(),
                kind: PackChangeKind::FeeCredit,
                amount_cents: -self.fee_credit,
                credit_minor: 0,
                credit_unit: self.unit.clone(),
                credit_precision: self.precision,
                promotional: false,
            });
        }
        changes
    }
}

const PAID: &str = "PAID";

/// What an immediate cancellation records to settle later the lines of invoices still drafts then.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct CancellationRecord {
    /// The instant the subscription's usage and credit end.
    pub settle_at: DateTime<Utc>,
    /// The period the cancellation ended early.
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
}

async fn record_notes(
    conn: &mut PgConn,
    kind: SettlingEnding,
    subscription_id: SubscriptionId,
    at: NaiveDate,
    notes: &[CreditNote],
) -> StoreResult<()> {
    if notes.is_empty() {
        return Ok(());
    }
    let ids: Vec<uuid::Uuid> = notes.iter().map(|note| note.id.as_uuid()).collect();
    diesel_models::subscription_events::SubscriptionEventRow::record_ending_notes(
        conn,
        kind,
        subscription_id,
        at,
        &ids,
    )
    .await?;
    Ok(())
}

fn midnight(day: NaiveDate) -> DateTime<Utc> {
    day.and_time(chrono::NaiveTime::MIN).and_utc()
}

/// The subtotal of an old pack line credited back: the unused time of what is left of it after
/// earlier notes, capped by its unused paid credit, both read against what the line still bills.
/// Included credit refunds nothing; a pack not yet paid (no grant) by time.
fn pack_fee_credit(
    line: &LineItem,
    grants: &[CustomerCreditGrantRow],
    change_date: NaiveDate,
    already_credited: i64,
) -> i64 {
    if line.credit_grant_included || line.taxable_amount <= 0 {
        return 0;
    }
    let billed = (line.amount_subtotal - already_credited).max(0);
    let unused_time = DayFraction::remaining(line.start_date, line.end_date, change_date);
    let paid = grants.iter().filter(|grant| grant.classification == PAID);
    let (unused, face) = paid.fold((0_i64, 0_i64), |(unused, face), grant| {
        (unused + grant.remaining.max(0), face + grant.amount)
    });
    if face > 0 {
        capped_pack_fee_credit(billed, unused_time, unused, face)
    } else {
        unused_time.prorate(billed)
    }
}

/// What an immediate plan change or cancellation at `at` stops of the old plan, by lineage.
#[derive(Debug, Clone)]
pub(crate) struct Ending {
    pub kind: PeriodCreditRemoval,
    /// The day fees are prorated from: the day itself counts as unused.
    pub at: NaiveDate,
    /// The instant the ending packs settle usage up to, and their credit is removed at.
    pub settle_at: DateTime<Utc>,
    /// Whether a shortfall that prepaid the period (invoices written before the true-up) is
    /// credited by the time left (cancellation only).
    pub credits_minimum: bool,
    /// The most each fixed fee gives back for the time left.
    pub fees: BTreeMap<String, FeeCap>,
    /// Packs tied to the period.
    pub tied_packs: HashSet<String>,
    /// Seats added mid-period are billed on lines naming only the plan component.
    pub plan_components: HashMap<String, PriceComponentId>,
    /// How far back, in months, a line billing it in advance may start.
    pub reach_months: u32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum FeeCap {
    /// The fee for the time left, as the change priced it.
    Remaining(i64),
    /// A full period's fee, prorated over the period of the first line that billed it.
    FullPeriod(i64),
}

impl Ending {
    fn of_change(prepared: &PreparedPlanChange, at: NaiveDate) -> Self {
        let carried: Vec<String> = prepared
            .carried
            .values()
            .map(|root| root.to_string())
            .collect();
        let mut ending = Self::of_components(
            PeriodCreditRemoval::PlanChange,
            &prepared.proration.lines,
            &prepared.subscription_details,
            &carried,
            at,
            midnight(at),
        );
        ending.end_add_ons(&prepared.ending_add_ons);
        ending
    }

    fn end_add_ons(&mut self, add_ons: &[SubscriptionAddOn]) {
        for add_on in add_ons.iter().filter(|a| is_period_tied(&a.fee)) {
            self.tied_packs.insert(add_on.lineage().to_string());
        }
        self.reach_months = add_ons
            .iter()
            .map(|a| a.period.as_months().max(0) as u32)
            .fold(self.reach_months, u32::max);
    }

    /// An immediate cancellation: every component and add-on ends. A true-up is never credited; a
    /// shortfall that prepaid the period before the true-up gives its unused days back like a fee.
    pub(crate) fn of_cancellation(
        proration: &ProrationResult,
        details: &SubscriptionDetails,
        at: NaiveDate,
        settle_at: DateTime<Utc>,
        credits_minimum: bool,
    ) -> Self {
        let mut ending = Self::of_components(
            PeriodCreditRemoval::Cancellation,
            &proration.lines,
            details,
            &[],
            at,
            settle_at,
        );
        ending.end_add_ons(&details.add_ons);
        ending.credits_minimum = credits_minimum;
        ending
    }

    fn of_components(
        kind: PeriodCreditRemoval,
        lines: &[ProrationLineItem],
        details: &SubscriptionDetails,
        carried: &[String],
        at: NaiveDate,
        settle_at: DateTime<Utc>,
    ) -> Self {
        let mut fees = BTreeMap::new();
        for line in lines.iter().filter(|l| l.amount_cents < 0) {
            if let Some(key) = &line.net_key
                && let FeeCap::Remaining(total) =
                    fees.entry(key.clone()).or_insert(FeeCap::Remaining(0))
            {
                *total -= line.amount_cents;
            }
        }
        let components = &details.price_components;
        Ending {
            kind,
            at,
            settle_at,
            credits_minimum: false,
            fees,
            tied_packs: components
                .iter()
                .filter(|c| is_period_tied(&c.fee))
                .map(|c| c.lineage().to_string())
                .filter(|lineage| !carried.contains(lineage))
                .collect(),
            plan_components: components
                .iter()
                .filter_map(|c| Some((c.lineage().to_string(), c.price_component_id?)))
                .collect(),
            reach_months: components
                .iter()
                .map(|c| c.period.as_months().max(0) as u32)
                .max()
                .unwrap_or(0),
        }
    }

    /// What the plan switch at `at` recorded: the components it closed whose lineage stops
    /// there (one carried over continues it), with their fees.
    async fn of_switch(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        at: NaiveDate,
        precision: u8,
    ) -> StoreResult<Self> {
        use diesel_models::subscription_components::SubscriptionComponentRow;
        let rows = SubscriptionComponentRow::list_component_history_for_period(
            conn,
            &subscription_id,
            at.pred_opt().unwrap_or(at),
            at.succ_opt().unwrap_or(at),
        )
        .await?;
        let mut ending = Ending {
            kind: PeriodCreditRemoval::PlanChange,
            at,
            settle_at: midnight(at),
            credits_minimum: false,
            fees: BTreeMap::new(),
            tied_packs: HashSet::new(),
            plan_components: HashMap::new(),
            reach_months: 0,
        };
        // The seats the change read before its new plan's seats were recorded; a switch of an
        // earlier release recorded none, its seats are read at the instant it ran.
        let switches =
            diesel_models::subscription_events::SubscriptionEventRow::list_settling_switch_records(
                conn,
                subscription_id,
                at,
            )
            .await?;
        let mut recorded_seats: HashMap<String, u32> = HashMap::new();
        for seats in switches.iter().filter_map(|(_, seats)| seats.as_ref()) {
            let seats: BTreeMap<String, u32> =
                serde_json::from_value(seats.clone()).map_err(|error| {
                    Report::new(StoreError::SerdeError(
                        "plan switch seats".to_owned(),
                        error,
                    ))
                })?;
            for (unit, count) in seats {
                recorded_seats.entry(unit).or_insert(count);
            }
        }
        let changed_at = switches
            .iter()
            .map(|(created_at, _)| *created_at)
            .max()
            .unwrap_or_else(|| midnight(at).naive_utc());
        for row in rows.into_iter().filter(|row| row.effective_to == Some(at)) {
            let mut component: SubscriptionComponent = row.try_into()?;
            if let SubscriptionFee::Slot {
                unit,
                initial_slots,
                ..
            } = &mut component.fee
            {
                *initial_slots = match recorded_seats.get(unit.as_str()) {
                    Some(count) => *count,
                    None => SlotTransactionRow::fetch_active_slots(
                        conn,
                        tenant_id,
                        subscription_id,
                        unit.clone(),
                        changed_at,
                    )
                    .await?
                    .unwrap_or(0)
                    .max(0) as u32,
                };
            }
            let root = component.lineage();
            if SubscriptionComponentRow::lineage_ended_on(conn, root).await? != Some(at) {
                continue;
            }
            let key = root.to_string();
            ending.reach_months = ending
                .reach_months
                .max(component.period.as_months().max(0) as u32);
            if let Some(plan_component) = component.price_component_id {
                ending.plan_components.insert(key.clone(), plan_component);
            }
            if is_period_tied(&component.fee) {
                ending.tied_packs.insert(key);
                continue;
            }
            ending.add_full_period_fee(key, &component.fee, &component.period, precision);
        }
        let add_ons = SubscriptionAddOnRow::list_add_on_history_for_period(
            conn,
            &subscription_id,
            at.pred_opt().unwrap_or(at),
            at.succ_opt().unwrap_or(at),
        )
        .await?;
        let continued: HashSet<SubscriptionAddOnId> = add_ons
            .iter()
            .filter(|row| row.effective_to.is_none_or(|to| to > at))
            .map(|row| row.lineage_id.unwrap_or(row.id))
            .collect();
        for row in add_ons
            .into_iter()
            .filter(|row| row.effective_to == Some(at))
        {
            let add_on: SubscriptionAddOn = row.try_into()?;
            if continued.contains(&add_on.lineage()) {
                continue;
            }
            let key = add_on.lineage().to_string();
            ending.reach_months = ending
                .reach_months
                .max(add_on.period.as_months().max(0) as u32);
            if is_period_tied(&add_on.fee) {
                ending.tied_packs.insert(key);
                continue;
            }
            let fee = scale_fee(&add_on.fee, add_on.quantity);
            ending.add_full_period_fee(key, &fee, &add_on.period, precision);
        }
        Ok(ending)
    }

    fn add_full_period_fee(
        &mut self,
        key: String,
        fee: &SubscriptionFee,
        period: &SubscriptionFeeBillingPeriod,
        precision: u8,
    ) {
        let full = component_advance_amount_cents(fee, period, precision);
        if full > 0
            && let FeeCap::FullPeriod(total) = self.fees.entry(key).or_insert(FeeCap::FullPeriod(0))
        {
            *total += full;
        }
    }
}

/// The invoices an ending is settled against.
#[derive(Debug, Clone)]
pub(crate) struct EndingScope {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: SubscriptionId,
    pub currency: String,
    /// Billing period starts in `[from, to)`.
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl EndingScope {
    /// The period of the change, reaching back one longest cadence: a quarterly pack of a monthly
    /// subscription was billed before it. None when the change is at the period's end.
    fn of_subscription(subscription: &Subscription, ending: &Ending) -> Option<Self> {
        let to = subscription
            .current_period_end
            .filter(|end| ending.at < *end)?;
        let from = ending
            .at
            .checked_sub_months(chrono::Months::new(ending.reach_months))
            .map_or(subscription.current_period_start, |from| {
                from.min(subscription.current_period_start)
            });
        Some(EndingScope {
            tenant_id: subscription.tenant_id,
            customer_id: subscription.customer_id,
            subscription_id: subscription.id,
            currency: subscription.currency.clone(),
            from,
            to,
        })
    }
}

/// What an immediate change would settle of the old plan, as a preview reads it.
#[derive(Debug, Clone, Default)]
pub(crate) struct OutgoingEstimate {
    pub packs: Vec<OutgoingPack>,
    /// Subtotal of the old fixed fees credited back.
    pub fee_credits: i64,
}

/// The lines a plan change already credited, with how much: the notes its switches recorded.
#[derive(Debug, Default)]
struct Settled {
    credited: HashMap<String, i64>,
}

impl Settled {
    async fn read(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        ctx: &PeriodCreditContext,
    ) -> StoreResult<Self> {
        use diesel_models::credit_notes::CreditNoteRow;
        use diesel_models::enums::CreditNoteStatus;
        let ours: HashSet<uuid::Uuid> =
            diesel_models::subscription_events::SubscriptionEventRow::list_ending_note_ids(
                conn,
                subscription_id,
            )
            .await?
            .into_iter()
            .collect();
        let mut credited = HashMap::new();
        if ours.is_empty() {
            return Ok(Settled { credited });
        }
        for invoice in &ctx.invoices {
            for note in CreditNoteRow::list_by_invoice_id(conn, tenant_id, invoice.id).await? {
                if !ours.contains(&note.id.as_uuid()) || note.status == CreditNoteStatus::Draft {
                    continue;
                }
                let counted = note.status == CreditNoteStatus::Finalized;
                let lines: Vec<LineItem> =
                    serde_json::from_value(note.line_items).map_err(|e| {
                        Report::new(StoreError::SerdeError(
                            "Failed to deserialize credit note line items".to_string(),
                            e,
                        ))
                    })?;
                for line in lines {
                    let amount = if counted {
                        line.amount_subtotal.saturating_neg().max(0)
                    } else {
                        0
                    };
                    *credited.entry(line.local_id).or_insert(0) += amount;
                }
            }
        }
        Ok(Settled { credited })
    }

    fn contains(&self, line: &LineItem) -> bool {
        self.credited.contains_key(&line.local_id)
    }
}

/// The old fees' credits: each line that billed the component in advance across the change gives
/// back its own unused time of what it still bills, at its own discount and tax (the note
/// reverses them as charged). A fee billed on several lines (seats added mid-period) comes back
/// from each, never above the fee for the time left, less what a change already credited.
fn fee_credit_items(
    ending: &Ending,
    ctx: &PeriodCreditContext,
    settled: &Settled,
) -> Vec<CreditLineItem> {
    let at = ending.at;
    let covers = |line: &LineItem| line.start_date <= at && line.end_date > at;
    let unused_of = |line: &LineItem| {
        let already = ctx
            .already_credited
            .get(&line.local_id)
            .copied()
            .unwrap_or(0);
        DayFraction::remaining(line.start_date, line.end_date, at)
            .prorate((line.amount_subtotal - already).max(0))
    };
    let mut items = Vec::new();
    for (key, cap) in &ending.fees {
        let mut billed: Vec<&LineItem> = ctx
            .all_lines
            .iter()
            .filter(|line| {
                line.kind == LineItemKind::Regular
                    && line.metric_id.is_none()
                    && line.amount_subtotal > 0
                    && covers(line)
                    && match ctx.line_lineage.get(&line.local_id) {
                        Some(lineage) => lineage == key,
                        None => {
                            line.sub_add_on_id.is_none()
                                && line.price_component_id.is_some()
                                && ending.plan_components.get(key).copied()
                                    == line.price_component_id
                        }
                    }
            })
            .collect();
        billed.sort_by_key(|line| line.start_date);
        let Some(first) = billed.first() else {
            continue;
        };
        let cap = match cap {
            FeeCap::Remaining(cap) => *cap,
            FeeCap::FullPeriod(full) => {
                DayFraction::remaining(first.start_date, first.end_date, at).prorate(*full)
            }
        };
        let consumed: i64 = billed
            .iter()
            .filter_map(|line| settled.credited.get(&line.local_id))
            .sum();
        let unused: Vec<(&LineItem, i64)> = billed
            .iter()
            .filter(|line| !settled.contains(line))
            .map(|line| (*line, unused_of(line)))
            .filter(|(_, unused)| *unused > 0)
            .collect();
        let cap = (cap - consumed).max(0);
        let total: i64 = unused.iter().map(|(_, unused)| unused).sum();
        // The cap is one half-up rounding of the fee for the time left, the lines' credits one
        // each: they differ by up to half a cent per extra line, which is no over-credit.
        let rounding = i64::try_from(billed.len() / 2).unwrap_or(i64::MAX);
        let within_cap = total <= cap.saturating_add(rounding);
        let mut left = total.min(cap);
        let count = unused.len();
        for (index, (line, amount)) in unused.into_iter().enumerate() {
            // Scaled down to the cap by each line's share, the last line taking the remainder.
            let amount = if within_cap {
                amount
            } else if index + 1 == count {
                left
            } else {
                DayFraction::new(amount, total).prorate(cap).min(left)
            };
            left -= amount;
            if amount > 0 {
                items.push(CreditLineItem::Amount {
                    local_id: line.local_id.clone(),
                    amount_cents: amount,
                });
            }
        }
    }
    items
}

/// The credit, on a cancellation, of a shortfall an invoice written before the true-up charged up
/// front and that prepaid its period: its days left, prorated once over the lines a product-scoped
/// minimum split it into, never above what earlier notes left of a line.
fn shortfall_credit_items(
    ending: &Ending,
    ctx: &PeriodCreditContext,
    settled: &Settled,
) -> Vec<CreditLineItem> {
    if !ending.credits_minimum {
        return Vec::new();
    }
    let at = ending.at;
    let charged_up_front = |line: &LineItem| {
        ctx.line_to_invoice
            .get(&line.local_id)
            .and_then(|id| ctx.invoices.iter().find(|invoice| invoice.id == *id))
            .is_some_and(|invoice| minimum::is_up_front(line, invoice.billing_period_start))
    };
    let current: Vec<&LineItem> = ctx
        .all_lines
        .iter()
        .filter(|line| {
            line.kind == LineItemKind::MinimumShortfall
                && line.start_date <= at
                && at < line.end_date
                && charged_up_front(line)
                && !settled.contains(line)
        })
        .collect();
    let mut items = Vec::new();
    let mut periods: Vec<(NaiveDate, NaiveDate)> = current
        .iter()
        .map(|line| (line.start_date, line.end_date))
        .collect();
    periods.sort();
    periods.dedup();
    for (start, end) in periods {
        let lines: Vec<&&LineItem> = current
            .iter()
            .filter(|line| (line.start_date, line.end_date) == (start, end))
            .collect();
        let billed: i64 = lines.iter().map(|line| line.amount_subtotal).sum();
        if billed <= 0 {
            continue;
        }
        let credited = DayFraction::remaining(start, end, at).prorate(billed);
        let exact: Vec<Decimal> = lines
            .iter()
            .map(|line| {
                Decimal::from(credited) * Decimal::from(line.amount_subtotal)
                    / Decimal::from(billed)
            })
            .collect();
        for (line, share) in lines.iter().zip(meteroid_tax::largest_remainder(&exact)) {
            let already = ctx
                .already_credited
                .get(&line.local_id)
                .copied()
                .unwrap_or(0);
            let amount = share.min((line.amount_subtotal - already).max(0));
            if amount > 0 {
                items.push(CreditLineItem::Amount {
                    local_id: line.local_id.clone(),
                    amount_cents: amount,
                });
            }
        }
    }
    items
}

/// The old packs tied to the period whose line covers the change and that no change settled yet,
/// by the denomination they funded. Packs carried over unchanged are left alone.
fn tied_pack_lines<'a>(
    ctx: &'a PeriodCreditContext,
    ending: &Ending,
    settled: &Settled,
) -> BTreeMap<String, Vec<(&'a LineItem, InvoiceId)>> {
    let at = ending.at;
    let mut by_denomination: BTreeMap<String, Vec<(&LineItem, InvoiceId)>> = BTreeMap::new();
    for line in &ctx.all_lines {
        let tied_line = line.kind == LineItemKind::Credit
            && line.amount_subtotal >= 0
            && line.start_date <= at
            && at < line.end_date
            && !settled.contains(line)
            && ctx
                .line_lineage
                .get(&line.local_id)
                .is_some_and(|key| ending.tied_packs.contains(key));
        let Some(invoice) = ctx
            .line_to_invoice
            .get(&line.local_id)
            .and_then(|id| ctx.invoices.iter().find(|invoice| invoice.id == *id))
            .filter(|_| tied_line)
        else {
            continue;
        };
        let denomination = line.credit_grant_unit.as_ref().map_or_else(
            || normalized_currency(&invoice.currency),
            |unit| normalized_currency(&unit.code),
        );
        by_denomination
            .entry(denomination)
            .or_default()
            .push((line, invoice.id));
    }
    by_denomination
}

/// Whether an invoice of the subscription not issued yet bills across `at`: its settlement waits.
async fn has_unissued_lines_across(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    at: NaiveDate,
) -> StoreResult<bool> {
    let drafts = diesel_models::invoices::InvoiceRow::list_unissued_started_by(
        conn,
        tenant_id,
        subscription_id,
        at,
    )
    .await?;
    for draft in drafts {
        let draft: Invoice = draft.try_into()?;
        if draft
            .line_items
            .iter()
            .any(|line| line.start_date <= at && at < line.end_date)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn of_line(grant: &CustomerCreditGrantRow, line: &LineItem, invoice: InvoiceId) -> bool {
    grant.invoice_id == Some(invoice)
        && grant.line_local_id.as_deref() == Some(line.local_id.as_str())
        && matches!(
            grant.source_kind.as_str(),
            source::CREDIT_PURCHASE | source::INCLUDED_CREDIT
        )
}

/// Whether an ending settles `grant`'s pack up to it now. A plan change removes the pack, so it
/// always does; a cancellation only to cap the fee credit by the unused paid credit: the rest ends
/// with the subscription and the sweep settles it once, late usage included.
fn settles_now(ending: &Ending, grant: &CustomerCreditGrantRow) -> bool {
    match ending.kind {
        PeriodCreditRemoval::PlanChange | PeriodCreditRemoval::LatePayment => true,
        PeriodCreditRemoval::Cancellation => grant.classification == PAID,
    }
}

/// The face of a paid grant the refunded money (`refunded_net`, at the grant's cost) un-bills.
fn refunded_face(grant: &CustomerCreditGrantRow, refunded_net: i64) -> i64 {
    let face = match grant.cost_minor {
        Some(cost) if cost > 0 => (Decimal::from(refunded_net) * Decimal::from(grant.amount)
            / Decimal::from(cost))
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .to_i64()
        .unwrap_or(refunded_net),
        _ => refunded_net,
    };
    face.clamp(0, grant.remaining.max(0))
}

/// What the last pass of a rolled-back preview computed: each pass starts afresh, so a pass that
/// read metering it was not served never leaves its figures behind.
struct PassResult<T>(std::sync::Mutex<Option<T>>);

impl<T> Default for PassResult<T> {
    fn default() -> Self {
        Self(std::sync::Mutex::new(None))
    }
}

impl<T> PassResult<T> {
    fn slot(&self) -> StoreResult<std::sync::MutexGuard<'_, Option<T>>> {
        self.0
            .lock()
            .map_err(|_| Report::new(StoreError::InvoiceComputationError))
    }

    fn clear(&self) -> StoreResult<()> {
        *self.slot()? = None;
        Ok(())
    }

    fn set(&self, value: T) -> StoreResult<()> {
        *self.slot()? = Some(value);
        Ok(())
    }

    fn take(&self) -> StoreResult<Option<T>> {
        Ok(self.slot()?.take())
    }
}

/// What an ending settles, under the locks it took.
struct PlannedEnding {
    lock: CustomerLock,
    ctx: PeriodCreditContext,
    fee_items: Vec<CreditLineItem>,
    packs: Vec<OutgoingPack>,
}

impl Services {
    /// Settles, inside the change's transaction, what the old plan billed for the rest of the
    /// period. Returns the credit notes issued. A change at the period's end settles nothing.
    pub(crate) async fn settle_outgoing_components_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        prepared: &PreparedPlanChange,
        change_date: NaiveDate,
    ) -> StoreResult<Vec<CreditNoteId>> {
        let ending = Ending::of_change(prepared, change_date);
        let Some(scope) =
            EndingScope::of_subscription(&prepared.subscription_details.subscription, &ending)
        else {
            return Ok(vec![]);
        };
        // Held to the commit: an invoice of the period issued meanwhile (which locks the customer
        // first) is either issued before, and credited now, or after, and sees this change.
        lock_customer_tx(conn, scope.tenant_id, scope.customer_id).await?;
        let notes = self.settle_ending_tx(conn, actor, &scope, &ending).await?;
        Ok(notes.into_iter().map(|note| note.id).collect())
    }

    /// Settles, inside the cancellation's transaction, what the subscription billed in advance for
    /// the rest of the period, exactly as an immediate plan change to nothing would: its fees and
    /// minimum by the time left, its period packs capped by their unused paid credit.
    pub(crate) async fn settle_cancelled_components_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        details: &SubscriptionDetails,
        proration: &ProrationResult,
        at: NaiveDate,
        settle_at: DateTime<Utc>,
    ) -> StoreResult<Vec<CreditNoteId>> {
        let prepaid = minimum::up_front_shortfalls_prepaid(conn, details).await?;
        let ending = Ending::of_cancellation(proration, details, at, settle_at, prepaid);
        let Some(scope) = EndingScope::of_subscription(&details.subscription, &ending) else {
            return Ok(vec![]);
        };
        lock_customer_tx(conn, scope.tenant_id, scope.customer_id).await?;
        let notes = self.settle_ending_tx(conn, actor, &scope, &ending).await?;
        Ok(notes.into_iter().map(|note| note.id).collect())
    }

    /// Settles what the immediate plan changes recorded for its subscriptions ended of the lines
    /// `invoice` bills, once issued: a period invoice still a draft at a change is credited as an
    /// issued one was then. True when a note was issued.
    pub(crate) async fn settle_plan_change_endings_for_invoice_tx(
        &self,
        conn: &mut PgConn,
        invoice: &Invoice,
    ) -> StoreResult<bool> {
        let span = |lines: &[LineItem]| {
            let from = lines.iter().map(|line| line.start_date).min()?;
            let to = lines.iter().map(|line| line.end_date).max()?;
            Some((from, to))
        };
        let mut billed: Vec<(SubscriptionId, NaiveDate, NaiveDate)> = Vec::new();
        match invoice.subscription_id {
            Some(subscription_id) => {
                if let Some((from, to)) = span(&invoice.line_items) {
                    billed.push((subscription_id, from, to));
                }
            }
            None => {
                use diesel_models::invoices::InvoiceRow;
                for child in
                    InvoiceRow::list_consolidated_children(conn, invoice.tenant_id, invoice.id)
                        .await?
                {
                    let child: Invoice = child.try_into()?;
                    if let (Some(subscription_id), Some((from, to))) =
                        (child.subscription_id, span(&child.line_items))
                    {
                        billed.push((subscription_id, from, to));
                    }
                }
            }
        }
        let mut issued = false;
        for (subscription_id, from, to) in billed {
            use diesel_models::subscription_events::SubscriptionEventRow;
            let switches =
                SubscriptionEventRow::list_settling_switches(conn, subscription_id, from, to)
                    .await?;
            for at in switches {
                let notes = self
                    .settle_plan_change_ending_tx(
                        conn,
                        &Actor::System,
                        invoice.tenant_id,
                        subscription_id,
                        at,
                    )
                    .await?;
                issued |= !notes.is_empty();
            }
            let cancellations =
                SubscriptionEventRow::list_settling_cancellations(conn, subscription_id, from, to)
                    .await?;
            for at in cancellations {
                let notes = self
                    .settle_cancellation_ending_tx(
                        conn,
                        &Actor::System,
                        invoice.tenant_id,
                        subscription_id,
                        at,
                    )
                    .await?;
                issued |= !notes.is_empty();
            }
        }
        Ok(issued)
    }

    /// Settles what the plan switch of `subscription_id` at `at` ended and no change settled yet.
    /// Idempotent: what a change already credited or removed is left as is.
    pub(crate) async fn settle_plan_change_ending_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        at: NaiveDate,
    ) -> StoreResult<Vec<CreditNote>> {
        let subscription = diesel_models::subscriptions::SubscriptionRow::get_subscription_by_id(
            conn,
            &tenant_id,
            subscription_id,
        )
        .await?
        .subscription;
        let precision =
            crate::constants::Currencies::resolve_currency_precision(&subscription.currency)
                .unwrap_or(2);
        let ending = Ending::of_switch(conn, tenant_id, subscription_id, at, precision).await?;
        let scope = EndingScope {
            tenant_id,
            customer_id: subscription.customer_id,
            subscription_id,
            currency: subscription.currency,
            from: at
                .checked_sub_months(chrono::Months::new(ending.reach_months.max(1)))
                .unwrap_or(at),
            to: at.succ_opt().unwrap_or(at),
        };
        let notes = self.settle_ending_tx(conn, actor, &scope, &ending).await?;
        record_notes(
            conn,
            SettlingEnding::PlanChange,
            subscription_id,
            at,
            &notes,
        )
        .await?;
        Ok(notes)
    }

    /// Settles what the immediate cancellation of `subscription_id` on `at` ended and no
    /// settlement credited yet: the lines of invoices still drafts when it ran, once issued.
    /// Idempotent.
    pub(crate) async fn settle_cancellation_ending_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        at: NaiveDate,
    ) -> StoreResult<Vec<CreditNote>> {
        let Some(record) =
            diesel_models::subscription_events::SubscriptionEventRow::settling_cancellation(
                conn,
                subscription_id,
                at,
            )
            .await?
        else {
            return Ok(vec![]);
        };
        let record: CancellationRecord = serde_json::from_value(record).map_err(|error| {
            Report::new(StoreError::SerdeError(
                "settling cancellation".to_owned(),
                error,
            ))
        })?;
        use crate::repositories::subscriptions::SubscriptionInterface;
        let details = self
            .store
            .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
            .await?;
        let precision = crate::constants::Currencies::resolve_currency_precision(
            &details.subscription.currency,
        )
        .unwrap_or(2);
        let proration = crate::services::subscriptions::terminate::cancellation_proration(
            &details,
            record.period_start,
            record.period_end,
            at,
            precision,
        );
        let prepaid = minimum::up_front_shortfalls_prepaid(conn, &details).await?;
        let ending = Ending::of_cancellation(&proration, &details, at, record.settle_at, prepaid);
        let scope = EndingScope {
            tenant_id,
            customer_id: details.subscription.customer_id,
            subscription_id,
            currency: details.subscription.currency.clone(),
            from: at
                .checked_sub_months(chrono::Months::new(ending.reach_months))
                .map_or(record.period_start, |from| from.min(record.period_start)),
            to: record.period_end,
        };
        let notes = self.settle_ending_tx(conn, actor, &scope, &ending).await?;
        record_notes(
            conn,
            SettlingEnding::Cancellation,
            subscription_id,
            at,
            &notes,
        )
        .await?;
        Ok(notes)
    }

    /// Records, on its CANCELLED event, that the immediate cancellation on `at` settles the lines
    /// it ended, now (`notes`) or once their invoice is issued.
    pub(crate) async fn record_settling_cancellation_tx(
        &self,
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        at: NaiveDate,
        record: &CancellationRecord,
        notes: &[CreditNoteId],
    ) -> StoreResult<()> {
        let mut details = serde_json::to_value(record).map_err(|error| {
            Report::new(StoreError::SerdeError(
                "settling cancellation".to_owned(),
                error,
            ))
        })?;
        if let Some(fields) = details.as_object_mut() {
            fields.insert("outgoing".to_owned(), serde_json::json!("settled"));
            fields.insert(
                "notes".to_owned(),
                serde_json::json!(
                    notes
                        .iter()
                        .map(|id| id.as_uuid().to_string())
                        .collect::<Vec<_>>()
                ),
            );
        }
        let recorded =
            diesel_models::subscription_events::SubscriptionEventRow::record_settling_cancellation(
                conn,
                subscription_id,
                at,
                details,
            )
            .await?;
        if recorded == 0 {
            log::error!(
                "the cancellation of subscription {subscription_id} on {at} has no CANCELLED event; \
                 its invoices issued later will not be credited"
            );
        }
        Ok(())
    }

    /// Settles every plan switch and immediate cancellation due for a check whose invoices may still
    /// hold something to settle (one an earlier release issued during a deploy, or a missed hook),
    /// `page` at a time until all were visited; one none of whose invoices is left unissued is
    /// completed. Idempotent.
    pub(crate) async fn settle_missed_plan_change_endings(&self, page: i64) -> StoreResult<usize> {
        let mut issued = 0;
        for kind in [SettlingEnding::PlanChange, SettlingEnding::Cancellation] {
            issued += self.settle_missed_endings(kind, page).await?;
        }
        Ok(issued)
    }

    async fn settle_missed_endings(&self, kind: SettlingEnding, page: i64) -> StoreResult<usize> {
        use diesel_models::subscription_events::SubscriptionEventRow;
        use scoped_futures::ScopedFutureExt;
        let now = Utc::now().naive_utc();
        let mut issued = 0;
        let mut after = None;
        loop {
            let pending = {
                let mut conn = self.store.get_conn().await?;
                SubscriptionEventRow::list_incomplete_settling_endings(
                    &mut conn,
                    kind,
                    after,
                    page.max(1),
                    now,
                )
                .await?
            };
            let Some(last) = pending.last() else {
                return Ok(issued);
            };
            after = Some((last.applies_to, last.subscription_id));
            for due in pending.into_iter().filter(|due| due.due) {
                // Metering is read before the customer lock, on a pass of its own.
                let mut conn = self.store.get_conn().await?;
                let outcome = self
                    .transaction_reading_usage_first(&mut conn, |services, conn| {
                        async move {
                            // Before what is creditable is read: an invoice an older release issues
                            // meanwhile is either seen issued here, or still a draft at completion.
                            lock_customer_tx(conn, due.tenant_id, due.customer_id).await?;
                            let notes = match kind {
                                SettlingEnding::PlanChange => {
                                    services
                                        .settle_plan_change_ending_tx(
                                            conn,
                                            &Actor::System,
                                            due.tenant_id,
                                            due.subscription_id,
                                            due.applies_to,
                                        )
                                        .await?
                                }
                                SettlingEnding::Cancellation => {
                                    services
                                        .settle_cancellation_ending_tx(
                                            conn,
                                            &Actor::System,
                                            due.tenant_id,
                                            due.subscription_id,
                                            due.applies_to,
                                        )
                                        .await?
                                }
                            };
                            if has_unissued_lines_across(
                                conn,
                                due.tenant_id,
                                due.subscription_id,
                                due.applies_to,
                            )
                            .await?
                            {
                                SubscriptionEventRow::record_settling_ending_check(
                                    conn,
                                    kind,
                                    due.subscription_id,
                                    due.applies_to,
                                    Utc::now().naive_utc(),
                                    due.invoices_changed,
                                )
                                .await?;
                            } else {
                                SubscriptionEventRow::mark_settling_ending_complete(
                                    conn,
                                    kind,
                                    due.subscription_id,
                                    due.applies_to,
                                )
                                .await?;
                            }
                            Ok::<_, Report<StoreError>>(notes.len())
                        }
                        .scope_boxed()
                    })
                    .await;
                match outcome {
                    Ok(count) => issued += count,
                    Err(error) => {
                        log::error!(
                            "{kind:?} of subscription {} on {} was not settled: {error:?}",
                            due.subscription_id,
                            due.applies_to
                        );
                        // A failing one backs off like any other.
                        SubscriptionEventRow::record_settling_ending_check(
                            &mut conn,
                            kind,
                            due.subscription_id,
                            due.applies_to,
                            Utc::now().naive_utc(),
                            false,
                        )
                        .await?;
                    }
                }
            }
        }
    }

    async fn settle_ending_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        scope: &EndingScope,
        ending: &Ending,
    ) -> StoreResult<Vec<CreditNote>> {
        let Some(planned) = self.plan_ending_tx(conn, scope, ending).await? else {
            return Ok(vec![]);
        };
        let reason = match ending.kind {
            PeriodCreditRemoval::PlanChange => PLAN_CHANGE_NOTE_REASON,
            PeriodCreditRemoval::Cancellation => CANCELLATION_NOTE_REASON,
            PeriodCreditRemoval::LatePayment => LATE_PAYMENT_NOTE_REASON,
        };
        let mut items = planned.fee_items;
        items.extend(
            planned
                .packs
                .iter()
                .filter(|pack| pack.fee_credit > 0)
                .map(|pack| CreditLineItem::Amount {
                    local_id: pack.line_local_id.clone(),
                    amount_cents: pack.fee_credit,
                }),
        );
        let notes = self
            .issue_period_credit_notes_tx(
                conn,
                actor,
                scope.tenant_id,
                &planned.ctx,
                items,
                reason,
                true,
                Some(ending.settle_at),
            )
            .await?;
        self.remove_outgoing_pack_credit_tx(conn, &planned.lock, &planned.packs, &notes, ending)
            .await?;
        Ok(notes)
    }

    /// The old packs' credit as a preview reads it: the usage up to the change is settled in a
    /// transaction rolled back, so the figures are exactly those the change would remove. Metering
    /// is read between passes, outside any transaction.
    pub(crate) async fn estimate_outgoing_packs(
        &self,
        prepared: &PreparedPlanChange,
        change_date: NaiveDate,
    ) -> StoreResult<OutgoingEstimate> {
        let ending = Ending::of_change(prepared, change_date);
        let Some(scope) =
            EndingScope::of_subscription(&prepared.subscription_details.subscription, &ending)
        else {
            return Ok(OutgoingEstimate::default());
        };
        let estimate = PassResult::default();
        let (scope, ending, slot) = (&scope, &ending, &estimate);
        let mut conn = self.store.get_conn().await?;
        let outcome = self
            .transaction_reading_usage_first(&mut conn, |services, conn| {
                use scoped_futures::ScopedFutureExt;
                async move {
                    slot.clear()?;
                    let estimate = match services.plan_ending_tx(conn, scope, ending).await? {
                        Some(planned) => OutgoingEstimate {
                            fee_credits: planned
                                .fee_items
                                .iter()
                                .map(|item| match item {
                                    CreditLineItem::Amount { amount_cents, .. } => *amount_cents,
                                    _ => 0,
                                })
                                .sum(),
                            packs: planned.packs,
                        },
                        None => OutgoingEstimate::default(),
                    };
                    slot.set(estimate)?;
                    Err::<(), _>(Report::new(StoreError::InvalidArgument(
                        "plan change preview rolled back".to_owned(),
                    )))
                }
                .scope_boxed()
            })
            .await;
        match (estimate.take()?, outcome) {
            (Some(estimate), _) => Ok(estimate),
            (None, Err(error)) => Err(error),
            (None, Ok(())) => Ok(OutgoingEstimate::default()),
        }
    }

    /// The adjustment invoice an immediate change would issue, priced after the old plan is
    /// settled (as the change does) in a nested transaction rolled back: the amount a checkout
    /// charges. Nothing it wrote survives. Outermost, its metering is read before the customer
    /// lock; nested in a caller's transaction, under the caller's locks.
    pub(crate) async fn dry_run_plan_change_adjustment_tx(
        &self,
        conn: &mut PgConn,
        prepared: &PreparedPlanChange,
        new_plan_version_id: common_domain::ids::PlanVersionId,
        change_date: NaiveDate,
    ) -> StoreResult<crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent> {
        self.dry_run_plan_change_adjustment_reads(conn, prepared, new_plan_version_id, change_date)
            .await
            .map(|(computed, _)| computed)
    }

    /// The metering an immediate plan change reads, read outside any transaction or lock: the
    /// checkout that applies it is served these under its locks. None for a free trial.
    pub(crate) async fn plan_change_checkout_usage(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        new_plan_version_id: common_domain::ids::PlanVersionId,
        change_date: NaiveDate,
    ) -> StoreResult<Option<UsageReads>> {
        let mut conn = self.store.get_conn().await?;
        let prepared = self
            .prepare_plan_change_readonly(
                &mut conn,
                subscription_id,
                tenant_id,
                new_plan_version_id,
                &[],
                change_date,
            )
            .await?;
        if prepared.is_free_trial() {
            return Ok(None);
        }
        let (_, reads) = self
            .dry_run_plan_change_adjustment_reads(
                &mut conn,
                &prepared,
                new_plan_version_id,
                change_date,
            )
            .await?;
        Ok(Some(reads))
    }

    async fn dry_run_plan_change_adjustment_reads(
        &self,
        conn: &mut PgConn,
        prepared: &PreparedPlanChange,
        new_plan_version_id: common_domain::ids::PlanVersionId,
        change_date: NaiveDate,
    ) -> StoreResult<(
        crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent,
        UsageReads,
    )> {
        let subscription = &prepared.subscription_details.subscription;
        let computed = PassResult::default();
        let slot = &computed;
        let (outcome, reads) = self
            .transaction_keeping_usage_reads(conn, |services, conn| {
                use scoped_futures::ScopedFutureExt;
                async move {
                    slot.clear()?;
                    services
                        .execute_plan_change_tx(
                            conn,
                            &Actor::System,
                            prepared,
                            subscription.id,
                            subscription.tenant_id,
                            new_plan_version_id,
                            change_date,
                        )
                        .await?;
                    let content = services
                        .compute_adjustment_invoice_content(
                            conn,
                            subscription.tenant_id,
                            subscription,
                            &prepared.subscription_details.customer,
                            &prepared.charges(),
                        )
                        .await?;
                    slot.set(content.computed)?;
                    Err::<(), _>(Report::new(StoreError::InvalidArgument(
                        "plan change preview rolled back".to_owned(),
                    )))
                }
                .scope_boxed()
            })
            .await?;
        match (computed.take()?, outcome) {
            (Some(computed), _) => Ok((computed, reads)),
            (None, Err(error)) => Err(error),
            (None, Ok(())) => Err(Report::new(StoreError::InvalidArgument(
                "the plan change preview computed nothing".to_owned(),
            ))),
        }
    }

    /// Locks the customer, the scope's invoices and the grants of each denomination the ended
    /// packs funded, in the credit lock order, then settles those packs' usage up to the change.
    /// None, without a lock, when nothing is left to settle.
    async fn plan_ending_tx(
        &self,
        conn: &mut PgConn,
        scope: &EndingScope,
        ending: &Ending,
    ) -> StoreResult<Option<PlannedEnding>> {
        let EndingScope {
            tenant_id,
            customer_id,
            subscription_id,
            ..
        } = *scope;
        let at = ending.settle_at;
        let now = Utc::now();

        // Usage is read before the grant locks; under the customer lock every caller holds, it is
        // served by a pass that ran before it (`transaction_reading_usage_first`).
        let unlocked =
            gather_period_credit_context(conn, tenant_id, subscription_id, scope.from, scope.to)
                .await?;
        let settled = Settled::read(conn, tenant_id, subscription_id, &unlocked).await?;
        let grants =
            CustomerCreditGrantRow::list_for_customer(conn, tenant_id, customer_id).await?;
        let mut owed_packs = tied_pack_lines(&unlocked, ending, &settled);
        // A pack whose credit is all gone has nothing left to settle.
        owed_packs.retain(|_, lines| {
            lines.retain(|(line, invoice)| {
                let mut of_this = grants
                    .iter()
                    .filter(|g| of_line(g, line, *invoice))
                    .peekable();
                of_this.peek().is_none()
                    || of_this.any(|grant| grant.remaining > 0 && settles_now(ending, grant))
            });
            !lines.is_empty()
        });
        if owed_packs.is_empty()
            && fee_credit_items(ending, &unlocked, &settled).is_empty()
            && shortfall_credit_items(ending, &unlocked, &settled).is_empty()
        {
            return Ok(None);
        }
        let mut usage = HashMap::new();
        for denomination in owed_packs.into_keys() {
            let read = read_ending_usage(
                &self.store,
                self.usage_client.as_ref(),
                conn,
                tenant_id,
                customer_id,
                &denomination,
                at,
                now,
            )
            .await?;
            usage.insert(denomination, read);
        }

        let lock = lock_customer_tx(conn, tenant_id, customer_id).await?;
        let ctx =
            gather_period_credit_context(conn, tenant_id, subscription_id, scope.from, scope.to)
                .await?;
        for invoice in &ctx.invoices {
            diesel_models::invoices::InvoiceRow::lock_by_id(conn, &lock, invoice.id).await?;
        }
        let settled = Settled::read(conn, tenant_id, subscription_id, &ctx).await?;
        let currency_precision =
            crate::constants::Currencies::resolve_currency_precision(&scope.currency).unwrap_or(2);

        let mut packs = Vec::new();
        for (denomination, lines) in tied_pack_lines(&ctx, ending, &settled) {
            let mut grants = lock_grants_tx(conn, &lock, &denomination).await?;
            let settling: Vec<&(&LineItem, InvoiceId)> = lines
                .iter()
                .filter(|(line, invoice)| {
                    grants.iter().any(|scoped| {
                        of_line(&scoped.grant, line, *invoice)
                            && scoped.grant.remaining > 0
                            && settles_now(ending, &scoped.grant)
                    })
                })
                .collect();
            let ending_ids: Vec<CreditGrantId> = grants
                .iter()
                .filter(|scoped| {
                    settling
                        .iter()
                        .any(|(line, invoice)| of_line(&scoped.grant, line, *invoice))
                })
                .map(|scoped| scoped.grant.id)
                .collect();
            settle_ending_grants_tx(
                &self.store,
                self.usage_client.as_ref(),
                conn,
                tenant_id,
                customer_id,
                &denomination,
                &mut grants,
                &ending_ids,
                at,
                now,
                usage.get(&denomination),
            )
            .await?;
            for (line, invoice_id) in lines {
                let line_grants: Vec<CustomerCreditGrantRow> = grants
                    .iter()
                    .filter(|scoped| of_line(&scoped.grant, line, invoice_id))
                    .map(|scoped| scoped.grant.clone())
                    .collect();
                let already = ctx
                    .already_credited
                    .get(&line.local_id)
                    .copied()
                    .unwrap_or(0);
                packs.push(OutgoingPack {
                    name: line.name.clone(),
                    invoice_id,
                    line_local_id: line.local_id.clone(),
                    unit: line.credit_grant_unit.clone(),
                    precision: line
                        .credit_grant_unit
                        .as_ref()
                        .map_or(currency_precision, |unit| unit.precision),
                    fee_credit: pack_fee_credit(line, &line_grants, ending.at, already),
                    grants: line_grants,
                });
            }
        }
        let mut fee_items = fee_credit_items(ending, &ctx, &settled);
        fee_items.extend(shortfall_credit_items(ending, &ctx, &settled));
        Ok(Some(PlannedEnding {
            lock,
            ctx,
            fee_items,
            packs,
        }))
    }

    /// Removes the old packs' paid credit as far as its fee came back, un-billed with the note. On a
    /// plan change every grant of the pack ends at the change: the rest pays the usage dated before
    /// it that arrives within the settlement grace, then lapses. On a cancellation the rest stays,
    /// to end with the subscription.
    async fn remove_outgoing_pack_credit_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        packs: &[OutgoingPack],
        notes: &[CreditNote],
        ending: &Ending,
    ) -> StoreResult<()> {
        let at = ending.settle_at;
        let mut touched = Vec::new();
        let mut tenant = None;
        for pack in packs {
            let credited = notes.iter().find_map(|note| {
                note.line_items
                    .iter()
                    .find(|line| {
                        note.invoice_id == pack.invoice_id && line.local_id == pack.line_local_id
                    })
                    .map(|line| (note.id, line.taxable_amount.saturating_neg().max(0)))
            });
            let ids: Vec<CreditGrantId> = pack.grants.iter().map(|grant| grant.id).collect();
            let Some(first) = pack.grants.first() else {
                continue;
            };
            tenant = Some(first.tenant_id);
            // The notes may have given value back to these grants: remove what is left now.
            for grant in CustomerCreditGrantRow::lock_by_ids(conn, lock, &ids).await? {
                let ends = ending.kind == PeriodCreditRemoval::PlanChange;
                if grant.remaining <= 0 && !ends {
                    continue;
                }
                let note = credited.map(|(note, _)| note);
                let refunded = match credited {
                    Some((_, net)) if grant.classification == PAID => refunded_face(&grant, net),
                    _ => 0,
                };
                if refunded == 0 && !ends {
                    continue;
                }
                let updated = credit::remove_period_credit_tx(
                    conn,
                    lock,
                    &grant,
                    refunded,
                    note,
                    at,
                    ending.kind,
                )
                .await?;
                touched.push(updated.account_id);
            }
        }
        if let Some(tenant) = tenant {
            credit::balance_changed_tx(
                conn,
                &*self.store.hooks.credit,
                tenant,
                &touched,
                Utc::now(),
            )
            .await?;
        }
        Ok(())
    }
}

impl Services {
    /// A period pack first funded after its period was invoiced: a balance note credits the fee
    /// share of the credit, bonus included, that period would have drawn on time (reversing as much
    /// of the paid credit), the rest lapses, dated `funded_at`. Idempotent: removed credit stays.
    pub(crate) async fn credit_back_late_period_packs_tx(
        &self,
        conn: &mut PgConn,
        invoice: &Invoice,
        funded_at: DateTime<Utc>,
        forgiveness: Option<Forgiveness>,
    ) -> StoreResult<()> {
        use crate::domain::credit::LedgerEntryKind;
        use crate::domain::price_components::CreditExpiry;
        use crate::services::credit::mint::{
            credit_line_source, credit_period_tx, credited_by_finalized_notes_tx,
        };
        use crate::services::subscriptions::late_pack::{LatePack, pack_face_drawn_on_time_tx};
        let tenant_id = invoice.tenant_id;
        let late_packs: Vec<&LineItem> = invoice
            .line_items
            .iter()
            .filter(|line| {
                line.kind == LineItemKind::Credit
                    && !line.credit_grant_included
                    && line.amount_subtotal > 0
                    && line.credit_grant_expiry == Some(CreditExpiry::BillingPeriod)
            })
            .collect();
        if late_packs.is_empty() {
            return Ok(());
        }
        // The payment paths reaching here hold the invoice, so its customer: this yields the proof.
        let lock = lock_customer_tx(conn, tenant_id, invoice.customer_id).await?;
        let owners = crate::services::credit::scope::LineOwners::load_tx(
            conn,
            tenant_id,
            &invoice.line_items,
        )
        .await?;
        let credited = credited_by_finalized_notes_tx(conn, invoice, None).await?;
        for line in late_packs {
            let period = credit_period_tx(conn, invoice, line).await?;
            let Some(subscription_id) = crate::services::credit::scope::line_subscription(
                line,
                invoice.subscription_id,
                &owners,
            ) else {
                continue;
            };
            let source_id = credit_line_source(invoice.id, &line.local_id);
            let Some(paid) = CustomerCreditGrantRow::find_by_source(
                conn,
                tenant_id,
                source::CREDIT_PURCHASE,
                source_id,
            )
            .await?
            else {
                continue;
            };
            let movements = diesel_models::customer_credit::CustomerCreditEntryRow::list_by_grant(
                conn, tenant_id, paid.id,
            )
            .await?
            .into_iter()
            .map(|entry| Ok((LedgerEntryKind::from_db_str(&entry.kind)?, entry)))
            .collect::<StoreResult<Vec<_>>>()?;
            // Late only when first funded after its period: a payment reversed and paid again
            // later re-funds a grant that was spendable on time. A grant a forgiveness minted was
            // spendable only from it, though its FUND is dated at the payment before it.
            let first_funded = movements
                .iter()
                .filter(|(kind, _)| *kind == LedgerEntryKind::Fund)
                .map(|(_, entry)| entry.occurred_at)
                .min()
                .map(|funded| match forgiveness {
                    Some(forgiven) if paid.created_at >= forgiven.recorded_at => {
                        funded.max(forgiven.at)
                    }
                    _ => funded,
                });
            if first_funded.is_none_or(|at| at < midnight(period.1)) {
                continue;
            }
            let bonus = CustomerCreditGrantRow::find_by_source(
                conn,
                tenant_id,
                source::CREDIT_PURCHASE,
                credit::bonus_source(source_id),
            )
            .await?;
            if paid.remaining <= 0 && bonus.as_ref().is_none_or(|bonus| bonus.remaining <= 0) {
                continue;
            }
            if period_still_billable_tx(
                conn,
                tenant_id,
                invoice.customer_id,
                subscription_id,
                period,
            )
            .await?
            {
                continue;
            }
            let denomination = line
                .credit_grant_unit
                .as_ref()
                .map_or(invoice.currency.as_str(), |unit| unit.code.as_str());
            let on_time = pack_face_drawn_on_time_tx(
                conn,
                tenant_id,
                &LatePack {
                    subscription_id,
                    period,
                    denomination,
                    paid: &paid,
                    bonus: bonus.as_ref(),
                },
            )
            .await?;
            // What it did pay while its period was still billable is not owed back.
            let bonus_movements = match &bonus {
                Some(bonus) => {
                    diesel_models::customer_credit::CustomerCreditEntryRow::list_by_grant(
                        conn, tenant_id, bonus.id,
                    )
                    .await?
                }
                None => Vec::new(),
            };
            let mut drawn = 0;
            for entry in movements
                .iter()
                .map(|(_, entry)| entry)
                .chain(&bonus_movements)
            {
                if matches!(
                    LedgerEntryKind::from_db_str(&entry.kind)?,
                    LedgerEntryKind::Draw | LedgerEntryKind::DrawReversal
                ) {
                    drawn -= entry.amount;
                }
            }
            let owed = (on_time - drawn).max(0);
            let left = line.amount_subtotal - credited.get(&line.local_id).copied().unwrap_or(0);
            let fee_credit = if left > 0 && owed > 0 {
                capped_pack_fee_credit(left, DayFraction::WHOLE, owed, paid.amount)
            } else {
                0
            };
            let credited_back = if fee_credit > 0 {
                let ctx = gather_period_credit_context(
                    conn,
                    tenant_id,
                    subscription_id,
                    period.0,
                    period.1,
                )
                .await?;
                let notes = self
                    .issue_period_credit_notes_tx(
                        conn,
                        &Actor::System,
                        tenant_id,
                        &ctx,
                        vec![CreditLineItem::Amount {
                            local_id: line.local_id.clone(),
                            amount_cents: fee_credit,
                        }],
                        LATE_PAYMENT_NOTE_REASON,
                        true,
                        Some(funded_at),
                    )
                    .await?;
                notes.iter().find_map(|note| {
                    note.line_items
                        .iter()
                        .find(|credited| credited.local_id == line.local_id)
                        .map(|credited| (note.id, credited.taxable_amount.saturating_neg().max(0)))
                })
            } else {
                None
            };
            let ids: Vec<CreditGrantId> = std::iter::once(paid.id)
                .chain(bonus.as_ref().map(|bonus| bonus.id))
                .collect();
            let mut touched = Vec::new();
            for grant in CustomerCreditGrantRow::lock_by_ids(conn, &lock, &ids).await? {
                let (note, refunded) = match credited_back {
                    Some((note, net)) if grant.classification == PAID => {
                        (Some(note), refunded_face(&grant, net))
                    }
                    _ => (None, 0),
                };
                let updated = credit::remove_period_credit_tx(
                    conn,
                    &lock,
                    &grant,
                    refunded,
                    note,
                    funded_at,
                    PeriodCreditRemoval::LatePayment,
                )
                .await?;
                touched.push(updated.account_id);
            }
            credit::balance_changed_tx(
                conn,
                &*self.store.hooks.credit,
                tenant_id,
                &touched,
                Utc::now(),
            )
            .await?;
        }
        Ok(())
    }
}

/// The note that forgave the rest of a pack invoice: when (`at`), and when it was recorded, so a
/// grant minted by it is told from one funded before.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Forgiveness {
    pub at: DateTime<Utc>,
    pub recorded_at: DateTime<Utc>,
}

impl Services {
    /// OQ11 for forgiven debt: a note that cancelled the rest of an invoice made it paid and funded
    /// the paid share of its packs, which a late pack credits back as a late payment does. Runs
    /// once the note committed (crediting issues notes of its own); idempotent, keyed by the note.
    pub async fn credit_back_forgiven_late_packs(
        &self,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> StoreResult<()> {
        use crate::domain::enums::InvoicePaymentStatus;
        use crate::services::credit::mint::funding_time;
        use diesel_models::credit_notes::CreditNoteRow;
        use diesel_models::enums::CreditNoteStatus;
        use diesel_models::invoices::InvoiceRow;
        use diesel_models::payments::PaymentTransactionRow;
        use scoped_futures::ScopedFutureExt;

        self.store
            .transaction(|conn| {
                async move {
                    let note = CreditNoteRow::find_by_id(conn, tenant_id, credit_note_id).await?;
                    let Some(forgiven_at) = note.finalized_at.filter(|_| {
                        note.status == CreditNoteStatus::Finalized
                            && note.debt_cancelled_amount_cents > 0
                    }) else {
                        return Ok(());
                    };
                    InvoiceRow::lock_customer_of(conn, tenant_id, note.invoice_id).await?;
                    let invoice: Invoice = InvoiceRow::find_by_id(conn, tenant_id, note.invoice_id)
                        .await?
                        .try_into()?;
                    if invoice.payment_status != InvoicePaymentStatus::Paid {
                        return Ok(());
                    }
                    // As the forgiveness funded it: from the money paid before, at the latest then.
                    let forgiven_at = forgiven_at.and_utc();
                    let paid_at = PaymentTransactionRow::last_settled_by_invoice_id(
                        conn, invoice.id, tenant_id,
                    )
                    .await?
                    .and_then(|payment| payment.processed_at)
                    .map_or(forgiven_at, |at| at.and_utc());
                    self.credit_back_late_period_packs_tx(
                        conn,
                        &invoice,
                        funding_time(&invoice, paid_at, forgiven_at),
                        Some(Forgiveness {
                            at: forgiven_at,
                            recorded_at: note.created_at.and_utc(),
                        }),
                    )
                    .await
                }
                .scope_boxed()
            })
            .await
    }
}

impl Services {
    /// `credit_back_forgiven_late_packs` for the forgiven purchases whose credit is due to lapse,
    /// should the note's own event have been missed; returns how many notes were looked at.
    pub async fn credit_back_forgiven_late_packs_due(&self, limit: i64) -> StoreResult<usize> {
        let mut conn = self.store.get_conn().await?;
        let notes =
            CustomerCreditGrantRow::list_forgiven_purchase_notes(&mut conn, Utc::now(), limit)
                .await?;
        drop(conn);
        for (tenant_id, credit_note_id) in &notes {
            if let Err(error) = self
                .credit_back_forgiven_late_packs(*tenant_id, *credit_note_id)
                .await
            {
                log::error!(
                    "late pack of forgiven note {credit_note_id} not credited back: {error:?}"
                );
            }
        }
        Ok(notes.len())
    }
}

/// Whether an invoice can still bill the usage of `subscription_id`'s `period`: the subscription
/// has not moved past it yet, or one of its drafts bills usage dated in it.
async fn period_still_billable_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    subscription_id: SubscriptionId,
    period: (NaiveDate, NaiveDate),
) -> StoreResult<bool> {
    use diesel_models::query::credit_usage::{ConsolidatedDrafts, list_draft_recurring_periods};
    let subscription = diesel_models::subscriptions::SubscriptionRow::get_subscription_by_id(
        conn,
        &tenant_id,
        subscription_id,
    )
    .await?
    .subscription;
    if !subscription.status.is_terminal() && subscription.current_period_start < period.1 {
        return Ok(true);
    }
    for draft in
        list_draft_recurring_periods(conn, tenant_id, customer_id, ConsolidatedDrafts::Exclude)
            .await?
            .into_iter()
            .filter(|draft| draft.subscription_id == subscription_id)
    {
        let lines: Vec<LineItem> = serde_json::from_value(draft.line_items).map_err(|error| {
            Report::new(StoreError::SerdeError(
                "Failed to deserialize line_items".to_owned(),
                error,
            ))
        })?;
        if lines.iter().any(|line| {
            crate::services::invoice_lines::component::is_usage_based_line(line)
                && period.0 <= line.start_date
                && line.start_date < period.1
        }) {
            return Ok(true);
        }
    }
    Ok(false)
}

impl crate::services::ServicesEdge {
    /// Settles the plan changes whose invoices were issued without it (by an earlier release
    /// during a deploy), `page` switches at a time. Returns the credit notes issued.
    pub async fn settle_missed_plan_change_endings(&self, page: i64) -> StoreResult<usize> {
        self.services.settle_missed_plan_change_endings(page).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::WalletId;

    fn grant(
        classification: &str,
        amount: i64,
        remaining: i64,
        cost: Option<i64>,
    ) -> CustomerCreditGrantRow {
        CustomerCreditGrantRow {
            id: CreditGrantId::new(),
            tenant_id: TenantId::new(),
            customer_id: CustomerId::new(),
            account_id: WalletId::new(),
            amount,
            remaining,
            priority: 100,
            classification: classification.to_owned(),
            effective_at: Utc::now(),
            expires_at: None,
            source_kind: source::CREDIT_PURCHASE.to_owned(),
            source_id: uuid::Uuid::now_v7(),
            invoice_id: None,
            line_local_id: None,
            credit_note_id: None,
            reason: "Credit purchased on a paid invoice".to_owned(),
            reference: None,
            created_by: None,
            created_at: Utc::now(),
            cost_minor: cost,
            cost_currency: None,
            rollover: None,
            subscription_id: None,
            application: crate::domain::credit::CreditApplication::Invoice
                .as_db_str()
                .to_owned(),
            ends_at: None,
            end_reason: None,
            lapses_at: None,
            pays_usage_from: None,
        }
    }

    fn ending(at: NaiveDate, key: &str, cap: i64) -> Ending {
        Ending {
            kind: PeriodCreditRemoval::PlanChange,
            at,
            settle_at: midnight(at),
            credits_minimum: false,
            fees: [(key.to_owned(), FeeCap::Remaining(cap))]
                .into_iter()
                .collect(),
            tied_packs: HashSet::new(),
            plan_components: HashMap::new(),
            reach_months: 1,
        }
    }

    fn seat_line(local_id: &str, subtotal: i64, start: NaiveDate) -> LineItem {
        LineItem {
            local_id: local_id.to_owned(),
            name: "Seats".to_owned(),
            charge_type: None,
            amount_subtotal: subtotal,
            tax_rate: Decimal::ZERO,
            taxable_amount: subtotal,
            tax_amount: 0,
            amount_total: subtotal,
            tax_details: vec![],
            quantity: None,
            unit_price: None,
            start_date: start,
            end_date: NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
            sub_lines: vec![],
            is_prorated: false,
            price_component_id: None,
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: None,
            metric_id: None,
            description: None,
            group_by_dimensions: None,
            kind: LineItemKind::Regular,
            credit_grant_amount: None,
            credit_grant_expiry: None,
            credit_grant_included: false,
            taxed_at_purchase: false,
            tax_inclusive_quote: None,
            credit_unit: None,
            credit_grant_unit: None,
            credit_grant_rollover: None,
            promotional_credit: None,
            subscription_id: None,
            usage_ends_at: None,
        }
    }

    #[test]
    fn each_billed_line_gives_back_its_own_unused_time() {
        // 10 seats billed April 1 for 300.00, 10 more added April 11 for 200.00 (20 of 30 days);
        // removed April 16: 300 × 15/30 and 200 × 15/20, never 300.00 from the newest line first.
        let day = |d| NaiveDate::from_ymd_opt(2026, 4, d).unwrap();
        let ctx = PeriodCreditContext {
            invoices: vec![],
            all_lines: vec![
                seat_line("first", 30_000, day(1)),
                seat_line("added", 20_000, day(11)),
            ],
            line_to_invoice: Default::default(),
            line_lineage: [("first", "seats"), ("added", "seats")]
                .into_iter()
                .map(|(line, key)| (line.to_owned(), key.to_owned()))
                .collect(),
            already_credited: Default::default(),
            returned: Default::default(),
        };
        let items = fee_credit_items(&ending(day(16), "seats", 30_000), &ctx, &Settled::default());
        let amounts: Vec<(String, i64)> = items
            .into_iter()
            .map(|item| match item {
                CreditLineItem::Amount {
                    local_id,
                    amount_cents,
                } => (local_id, amount_cents),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            amounts,
            vec![("first".to_owned(), 15_000), ("added".to_owned(), 15_000)]
        );
    }

    #[test]
    fn each_line_gives_back_its_own_rounded_share_under_the_rounded_cap() {
        // 1 seat billed October 1 for 10.00, 9 more added October 18 for 40.65 (14 of 31 days)
        // and credited back by the change, which priced the 10 seats left at 45.16: the first
        // seat's own 10.00 × 14/31 = 4.52, not the 4.51 left under the cap rounded once.
        let day = |d| NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let ctx = PeriodCreditContext {
            invoices: vec![],
            all_lines: vec![
                LineItem {
                    end_date: end,
                    ..seat_line("first", 1_000, day(1))
                },
                LineItem {
                    end_date: end,
                    ..seat_line("added", 4_065, day(18))
                },
            ],
            line_to_invoice: Default::default(),
            line_lineage: [("first", "seats"), ("added", "seats")]
                .into_iter()
                .map(|(line, key)| (line.to_owned(), key.to_owned()))
                .collect(),
            already_credited: Default::default(),
            returned: Default::default(),
        };
        let settled = Settled {
            credited: [("added".to_owned(), 4_065)].into_iter().collect(),
        };
        let amounts: Vec<(String, i64)> =
            fee_credit_items(&ending(day(18), "seats", 4_516), &ctx, &settled)
                .into_iter()
                .map(|item| match item {
                    CreditLineItem::Amount {
                        local_id,
                        amount_cents,
                    } => (local_id, amount_cents),
                    _ => unreachable!(),
                })
                .collect();
        assert_eq!(amounts, vec![("first".to_owned(), 452)]);
    }

    #[test]
    fn discount_and_tax_never_change_the_share_credited() {
        // The note credits subtotals; it scales each line's discount and tax down with them.
        // 100.00 less a 20.00 coupon taxed 20%, and 60.00 taxed 10%; half the period left,
        // capped at 64.00 of the 80.00 unused: 64 × 50/80 and the remainder.
        let day = |d| NaiveDate::from_ymd_opt(2026, 4, d).unwrap();
        let discounted = LineItem {
            name: "Platform Fee".to_owned(),
            tax_rate: Decimal::new(20, 2),
            taxable_amount: 8_000,
            tax_amount: 1_600,
            amount_total: 9_600,
            ..seat_line("discounted", 10_000, day(1))
        };
        let taxed = LineItem {
            name: "Platform Fee".to_owned(),
            tax_rate: Decimal::new(10, 2),
            tax_amount: 600,
            amount_total: 6_600,
            ..seat_line("taxed", 6_000, day(1))
        };
        let ctx = PeriodCreditContext {
            invoices: vec![],
            all_lines: vec![discounted, taxed],
            line_to_invoice: Default::default(),
            line_lineage: [("discounted", "platform"), ("taxed", "platform")]
                .into_iter()
                .map(|(line, key)| (line.to_owned(), key.to_owned()))
                .collect(),
            already_credited: Default::default(),
            returned: Default::default(),
        };
        let amounts: Vec<(String, i64)> = fee_credit_items(
            &ending(day(16), "platform", 6_400),
            &ctx,
            &Settled::default(),
        )
        .into_iter()
        .map(|item| match item {
            CreditLineItem::Amount {
                local_id,
                amount_cents,
            } => (local_id, amount_cents),
            _ => unreachable!(),
        })
        .collect();
        assert_eq!(
            amounts,
            vec![
                ("discounted".to_owned(), 4_000),
                ("taxed".to_owned(), 2_400)
            ]
        );
    }

    #[test]
    fn refunded_money_un_bills_the_paid_face_at_its_cost() {
        assert_eq!(
            refunded_face(&grant(PAID, 50_000, 30_000, Some(50_000)), 25_000),
            25_000
        );
        // Never more than is left.
        assert_eq!(
            refunded_face(&grant(PAID, 50_000, 10_000, Some(50_000)), 25_000),
            10_000
        );
        // Units bought at 1 cent each: 10,000 cents un-bill 1,000,000 units.
        assert_eq!(
            refunded_face(&grant(PAID, 5_000_000, 2_000_000, Some(50_000)), 10_000),
            1_000_000
        );
    }
}
