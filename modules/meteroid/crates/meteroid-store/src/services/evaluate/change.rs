//! What a change owes from the facts it wrote: each fee it touched, evaluated at its instant,
//! reconciled against what the documents billed (R-080). Owed goes on an adjustment invoice, owed
//! back on notes, latest lines first or by unused time past an ending (R-077, R-078).

use std::collections::{BTreeSet, HashMap};

use billing_core::charge::{ChargeKey, ChargeKind, Fee};
use billing_core::evaluate::{Charge, DocumentSlot, Evaluation, evaluate};
use billing_core::facts::{Billing, DatedFact, Facts, FeeFact, SeatStep};
use billing_core::issued::IssuedLine;
use billing_core::reconcile::{Adjustment, Expected, TakeBack, reconcile};
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{BaseId, InvoiceId, SlotTransactionId, SubscriptionId, TenantId};
use diesel_models::enums::SlotTransactionStatusEnum;
use diesel_models::invoices::InvoiceRow;
use diesel_models::slot_transactions::SlotTransactionRow;
use error_stack::Report;
use rust_decimal::Decimal;

use super::bill::{Decision, as_written, decide, lineages_of, midnight, render, row_of};
use super::issued::{Issued, load_billed, noted_from};
use super::load::{Overlay, load_facts_with};
use super::pricer::{StoreFee, StorePricer};
use crate::StoreResult;
use crate::domain::entity_activity::Actor;
use crate::domain::enums::CreditNoteOrigin;
use crate::domain::subscription_changes::ProrationResult;
use crate::domain::{Customer, Invoice, InvoiceStatusEnum, LineItem, Subscription};
use crate::errors::StoreError;
use crate::repositories::credit_notes::CreditLineItem;
use crate::services::Services;
use crate::services::clients::usage::UsageClient;
use crate::services::invoice_lines::fees::pricing_error;
use crate::services::invoices::AdjustmentInvoiceContent;
use crate::services::subscriptions::amendment::{
    PeriodCreditContext, load_credit_matching_context,
};
use crate::services::subscriptions::proration::is_period_tied;
use crate::store::PgConn;

/// What reconcile owes back beyond what a charge's lines have left (R-085): settled by the notes
/// that took it first, counted so support sees why a refund is below proration.
static RECONCILE_SHORT: std::sync::LazyLock<opentelemetry::metrics::Counter<u64>> =
    std::sync::LazyLock::new(|| {
        common_logging::GLOBAL_METER
            .u64_counter("billing_reconcile_short_minor")
            .with_description(
                "What reconcile owes back beyond what a charge's lines have left, in minor units",
            )
            .build()
    });

pub(in crate::services) fn record_short(tenant: TenantId, change: &'static str, amount: i64) {
    RECONCILE_SHORT.add(
        u64::try_from(amount).unwrap_or(0),
        &[
            common_logging::KeyValue::new("tenant", tenant.to_string()),
            common_logging::KeyValue::new("change", change),
        ],
    );
}

/// How a change takes back what its fees billed beyond what the facts now owe them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::services) enum GiveBack {
    /// From the latest lines first (R-078).
    LatestFirst,
    /// From each line by its unused time past the change's day, the rest latest first (R-077).
    Unused,
}

/// Which fees a change reconciles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::services) enum Touched {
    /// Those whose rows or seats start or stop on its day.
    Changed,
    /// Every fee in force on its day: the subscription ends there.
    Ended,
}

/// A change to a subscription's facts, already written, dated `on` and read at `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::services) struct Change {
    pub on: NaiveDate,
    pub at: DateTime<Utc>,
    pub touched: Touched,
    pub give_back: GiveBack,
    /// Whether the packs it starts are bought on its adjustment invoice.
    pub bills_packs: bool,
    /// Whether it writes documents of its own; else its renewal bills what it owes.
    pub documents: bool,
    /// What a take-back beyond a line's rest is counted under (R-085).
    pub label: &'static str,
}

impl Change {
    /// Fee rows or seats written from `on`, read at its midnight.
    pub(in crate::services) fn of_rows(
        on: NaiveDate,
        give_back: GiveBack,
        label: &'static str,
    ) -> Self {
        Change {
            on,
            at: midnight(on),
            touched: Touched::Changed,
            give_back,
            bills_packs: true,
            documents: true,
            label,
        }
    }

    /// The same, read now: settled again once a draft of its period is issued, against the lines
    /// and notes written since, the later changes' included (R-080).
    pub(in crate::services) fn read_now(self) -> Self {
        Change {
            at: common_clock::now().max(self.at),
            ..self
        }
    }

    /// The same, its packs left out: an amendment bills no pack it adds before its renewal.
    pub(in crate::services) fn without_packs(self) -> Self {
        Change {
            bills_packs: false,
            ..self
        }
    }

    /// The same, writing no document: a change taking effect on a renewal it leaves to bill.
    pub(in crate::services) fn without_documents(self) -> Self {
        Change {
            documents: false,
            ..self
        }
    }

    /// The subscription ending on `on` at `at`: each fee gives back its unused time.
    pub(in crate::services) fn of_ending(on: NaiveDate, at: DateTime<Utc>) -> Self {
        Change {
            on,
            at,
            touched: Touched::Ended,
            give_back: GiveBack::Unused,
            bills_packs: false,
            documents: false,
            label: "ending",
        }
    }
}

/// What a change owes and owes back.
#[derive(Debug, Default)]
pub(in crate::services) struct ChangeDocuments {
    /// The adjustment invoice's lines, in the figures the fees are priced in, neither taxed nor
    /// covered by credit.
    pub bills: Vec<LineItem>,
    /// What notes take back, each with the invoice that billed its line.
    pub credits: Vec<(InvoiceId, CreditLineItem)>,
    /// What the adjustment bills, before tax.
    pub charged: i64,
    /// What the notes take back of issued lines, net.
    pub credited: i64,
}

/// The fees `change` reconciles among `facts`. A row cut by a dated price (R-097) is one row: its
/// parts' meeting day changes nothing.
fn touched(facts: &Facts<StoreFee>, change: &Change) -> BTreeSet<Fee> {
    let on = change.on;
    let continued = |row: &FeeFact<StoreFee>| {
        facts
            .fees
            .iter()
            .any(|other| !std::ptr::eq(other, row) && other.id == row.id && other.to == Some(on))
    };
    let cut = |row: &FeeFact<StoreFee>| {
        facts
            .fees
            .iter()
            .any(|other| !std::ptr::eq(other, row) && other.id == row.id && other.from == on)
    };
    match change.touched {
        Touched::Changed => facts
            .fees
            .iter()
            .filter(|row| (row.from == on && !continued(row)) || (row.to == Some(on) && !cut(row)))
            .map(|row| row.lineage.clone())
            .chain(
                facts
                    .seats
                    .iter()
                    .filter(|step| step.at == on)
                    .map(|step| step.lineage.clone()),
            )
            .collect(),
        // A one-time row a change closed on its day lives that day (R-093); an ending reaches
        // every fee from its day on, one a later change opened too, when backdated before it.
        Touched::Ended => facts
            .fees
            .iter()
            .filter(|row| row.window().contains(on) || row.from > on)
            .map(|row| row.lineage.clone())
            .collect(),
    }
}

/// Seats of a slot fee bought on a change's day and not paid yet: evaluated as held from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::services) struct Bought {
    pub lineage: Fee,
    pub seats: u64,
}

/// What a change is evaluated with beyond the facts written: seats it buys, not written yet, and
/// the fees it is restricted to (those it buys), all it touches when empty.
#[derive(Clone, Debug, Default)]
pub(in crate::services) struct Hypothesis {
    pub bought: Option<Bought>,
    pub only: BTreeSet<Fee>,
}

/// `facts` with `bought` held from `on`, on top of the seats its fee holds that day.
fn with_bought(facts: &mut Facts<StoreFee>, bought: &Bought, on: NaiveDate) {
    let (held, removed_at_renewal) = facts.seats_on(&bought.lineage, on).map_or((0, 0), |step| {
        let today = if step.at == on {
            step.removed_at_renewal
        } else {
            0
        };
        (step.quantity, today)
    });
    facts
        .seats
        .retain(|step| !(step.lineage == bought.lineage && step.at == on));
    facts.seats.push(SeatStep {
        id: SlotTransactionId::new(),
        lineage: bought.lineage.clone(),
        at: on,
        quantity: held.saturating_add(bought.seats),
        removed_at_renewal,
    });
}

/// The invoices of seat purchases still paying: their seats are no fact, so neither are they.
async fn pending_purchases(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<Vec<InvoiceId>> {
    Ok(SlotTransactionRow::list_by_subscription_id(
        conn,
        tenant_id,
        subscription_id,
        None,
        Some(SlotTransactionStatusEnum::Pending),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?
    .into_iter()
    .filter_map(|transaction| transaction.invoice_id)
    .collect())
}

/// What a change bills and takes back, by charge and by issued line.
#[derive(Debug, Default, PartialEq, Eq)]
struct Planned {
    /// Owed on the change's own documents, by charge.
    bills: Vec<(ChargeKey, i64)>,
    /// Taken back from issued lines, in the figures they were priced in.
    credits: Vec<(String, i64)>,
    /// Owed back beyond what a charge's lines have left (R-085).
    short: Vec<(ChargeKey, i64)>,
}

/// Each fixed charge of the `touched` fees from the change on, against the lines billing it: owed,
/// on the change's `own` documents up to what it puts there; owed back, from the lines, a pack's
/// left to its credit's settlement (OQ1). A period the change ends entirely owes nothing (R-080).
#[allow(clippy::too_many_arguments)]
fn plan(
    evaluation: &Evaluation,
    issued: &[IssuedLine],
    touched: &BTreeSet<Fee>,
    change: &Change,
    own: &[DocumentSlot],
    watermark: Option<NaiveDate>,
    noted: &HashMap<String, Vec<(NaiveDate, i64)>>,
    read: &[UsagePeriod],
    drafts: &BTreeSet<String>,
    ending: Option<NaiveDate>,
) -> Planned {
    let on = change.on;
    // A line of a period the watermark settled is no charge's, whatever longer period holds it.
    let issued: Vec<IssuedLine> = issued
        .iter()
        .filter(|line| !evaluation.settles(line))
        .cloned()
        .collect();
    let issued = issued.as_slice();
    let fixed = |kind: &ChargeKind| matches!(kind, ChargeKind::Fee | ChargeKind::Credit);
    // An ending owes, of a usage line billed past its cutoff, what its period's usage up to the
    // cutoff comes to, read whole: the rest comes back.
    let mut expected: Vec<Expected> = read
        .iter()
        .map(|period| Expected {
            key: period.key.clone(),
            amount: evaluation
                .charges
                .iter()
                .filter(|charge| period.holds(&charge.key) && !charge.is_commitment())
                .map(|charge| charge.amount)
                .sum(),
        })
        .collect();
    // A change writing no document leaves the days from its own to the renewal that bills them.
    let renewal_bills = |charge: &Charge| {
        !change.documents && change.touched == Touched::Changed && charge.window.0 >= on
    };
    // The usage a change reads is what it ends on its day's invoice, written before it (R-011).
    let riding_usage = |charge: &Charge| {
        change.touched == Touched::Changed
            && matches!(charge.key.kind, ChargeKind::Usage { .. })
            && charge.document == DocumentSlot::Recurring { period_start: on }
    };
    for charge in &evaluation.charges {
        let key = &charge.key;
        if !touched.contains(&key.fee)
            || !(fixed(&key.kind) || riding_usage(charge))
            || renewal_bills(charge)
            || charge.is_commitment()
            || (key.from < on && key.until <= on && !riding_usage(charge))
        {
            continue;
        }
        match expected.iter_mut().find(|e| e.key == *key) {
            Some(e) => e.amount += charge.amount,
            None => expected.push(Expected {
                key: key.clone(),
                amount: charge.amount,
            }),
        }
    }
    let mut ended: Vec<Expected> = Vec::new();
    for line in issued {
        // A charge the change ends from its period's start owes nothing. One whose period a
        // change folded into another (no charge has its key) keeps the worth of its days still
        // billed at its price, by a charge quoted over its period: whichever change reads it.
        let orphan = !evaluation
            .charges
            .iter()
            .any(|charge| charge.key == line.key);
        // A one-time fee's line, keyed on its day, is given back by an ending alone (R-093, R-016).
        let one_time = line.key.from == line.key.until && change.touched == Touched::Changed;
        let unowed = !line.voided
            && !one_time
            && touched.contains(&line.key.fee)
            && line.key.kind == ChargeKind::Fee
            && (line.key.from >= on || orphan)
            && !expected
                .iter()
                .chain(ended.iter().filter(|e| e.key != line.key))
                .any(|e| line.bills(&e.key));
        if unowed {
            let kept = if orphan {
                let days = (line.until - line.from).num_days();
                let covered = (0..days)
                    .map(|n| line.from + chrono::Days::new(n as u64))
                    .filter(|day| {
                        evaluation.charges.iter().any(|charge| {
                            charge.key.fee == line.key.fee
                                && charge.key.kind == line.key.kind
                                && charge.nominal.0 <= line.from
                                && line.until <= charge.nominal.1
                                && charge.window.0 <= *day
                                && *day < charge.window.1
                        })
                    })
                    .count();
                let billed = line.quoted_of(line.subtotal);
                billing_core::DayFraction::new(i64::try_from(covered).unwrap_or(0), days.max(1))
                    .prorate(billed)
            } else {
                0
            };
            match ended.iter_mut().find(|e| e.key == line.key) {
                Some(e) => e.amount += kept,
                None => ended.push(Expected {
                    key: line.key.clone(),
                    amount: kept,
                }),
            }
        }
    }
    expected.extend(ended);

    let take_back = match change.give_back {
        GiveBack::LatestFirst => TakeBack::LatestFirst,
        GiveBack::Unused => TakeBack::Unused(on),
    };
    // A line bills one charge it falls in, a period the change leaves behind included: a month's
    // line under a year's charge of the same fee (a change of cadence) is the month's, never
    // counted for the year too (R-080).
    let periods: Vec<&ChargeKey> = evaluation
        .charges
        .iter()
        .filter(|charge| touched.contains(&charge.key.fee) && fixed(&charge.key.kind))
        .map(|charge| &charge.key)
        .chain(expected.iter().map(|e| &e.key))
        .collect();
    let owner = |line: &IssuedLine| {
        billing_core::evaluate::owner(line, periods.iter().copied(), &evaluation.charges)
    };
    let mut planned = Planned::default();
    for charge in &expected {
        let lines: Vec<IssuedLine> = issued
            .iter()
            .filter(|line| line.bills(&charge.key) && owner(line) == Some(&charge.key))
            .cloned()
            .collect();
        let mine: Vec<&Charge> = evaluation
            .charges
            .iter()
            .filter(|c| c.key == charge.key && own.contains(&c.document))
            .collect();
        // Its own share, and what notes took back from the charge's lines that it owes again:
        // no other document bills that back.
        let given_back: i64 = lines
            .iter()
            .filter(|line| !line.voided)
            .map(|line| line.corrected)
            .sum();
        let cap: i64 = mine.iter().map(|c| c.amount).sum::<i64>() + given_back;
        // An included pack bills nothing and grants its credit when issued.
        let grants = mine
            .iter()
            .any(|c| c.credit_grant.is_some_and(|grant| grant.amount > 0));
        let billed_here = change.bills_packs || charge.key.kind != ChargeKind::Credit;
        if billed_here
            && !mine.is_empty()
            && let Decision::Bill(owed) = decide(
                charge,
                &lines,
                watermark.map(|w| {
                    (
                        w,
                        owed_since(evaluation, &charge.key, w) + noted_from(noted, &lines, w),
                    )
                }),
                false,
            )
        {
            let amount = owed.min(cap);
            if amount > 0 || (amount == 0 && grants) {
                planned.bills.push((charge.key.clone(), amount));
                continue;
            }
        }
        if charge.key.kind == ChargeKind::Credit {
            continue;
        }
        // Usage billed past a cutoff has no unused time: it comes back from the latest lines. A
        // fee an ending cut gives back its unused days past it whichever change settles it, so a
        // void's share stays the one the ending forgave (R-096).
        let take_back = match charge.key.kind {
            ChargeKind::Usage { .. } => TakeBack::LatestFirst,
            _ => match ending.filter(|end| {
                change.touched == Touched::Changed
                    && charge.key.from < *end
                    && *end < charge.key.until
                    && evaluation
                        .charges
                        .iter()
                        .any(|c| c.key == charge.key && c.window.1 == *end)
            }) {
                Some(end) => TakeBack::Unused(end),
                None => take_back,
            },
        };
        let adjustments = reconcile(std::slice::from_ref(charge), &lines, take_back);
        // A share falling on a draft's line waits for that draft to be issued (R-078): the
        // charge's other lines wait with it, as their share is split against the draft's.
        let waits = adjustments.iter().any(|adjustment| {
            matches!(adjustment, Adjustment::Credit { line, .. } if drafts.contains(line))
        });
        for adjustment in adjustments {
            match adjustment {
                Adjustment::Credit { .. } if waits => {}
                Adjustment::Credit { line, amount } => planned.credits.push((line, amount)),
                Adjustment::Short { key, amount } => planned.short.push((key, amount)),
                Adjustment::Bill { .. } => {}
            }
        }
    }
    planned
}

/// What the charges of `key` due on or after the watermark `w` owe.
fn owed_since(evaluation: &Evaluation, key: &ChargeKey, w: NaiveDate) -> i64 {
    evaluation
        .charges
        .iter()
        .filter(|charge| {
            charge.key == *key && !charge.is_commitment() && charge.due_at >= midnight(w)
        })
        .map(|charge| charge.amount)
        .sum()
}

/// The documents a change writes itself: the adjustment invoice of its day, and the recurring
/// invoice of the period it opens when that was written before it, voided or not (R-011, R-051,
/// R-096: a void forgives what it billed, never what is added after it), or when no
/// renewal will write it, the stored period running past the change (a change of cadence on a
/// boundary of its new grid only). An ending's charges are its final invoice's.
async fn own_documents(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    change: &Change,
) -> StoreResult<Vec<DocumentSlot>> {
    if !change.documents {
        return Ok(Vec::new());
    }
    let mut own = vec![DocumentSlot::Adjustment { on: change.on }];
    let written =
        InvoiceRow::find_existing_recurring_invoice(conn, tenant_id, subscription_id, change.on)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    let opened = match written {
        Some(_) => true,
        None => diesel_models::subscriptions::SubscriptionRow::get_subscription_by_id(
            conn,
            &tenant_id,
            subscription_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .subscription
        .current_period_end
        .is_some_and(|end| end > change.on),
    };
    if opened {
        own.push(DocumentSlot::Recurring {
            period_start: change.on,
        });
    }
    Ok(own)
}

/// The charge of a usage line an ending reads whole, its period's usage read up to the cutoff.
#[derive(Clone, Debug, PartialEq, Eq)]
struct UsagePeriod {
    key: ChargeKey,
}

impl UsagePeriod {
    /// Whether `key` is a charge of its usage within its period: the ending cuts the last one.
    fn holds(&self, key: &ChargeKey) -> bool {
        key.fee == self.key.fee
            && key.kind == self.key.kind
            && key.denomination == self.key.denomination
            && self.key.from <= key.from
            && key.until <= self.key.until
    }
}

/// `evaluation` with the usage of each period a touched fee's issued usage line billed past the
/// ending's cutoff, and those periods whose every window was read: the others are left alone.
async fn with_usage_past_the_cutoff(
    client: &dyn UsageClient,
    loaded: &super::load::LoadedFacts,
    evaluation: Evaluation,
    issued: &[IssuedLine],
    touched: &BTreeSet<Fee>,
    change: &Change,
) -> StoreResult<(Evaluation, Vec<UsagePeriod>)> {
    let mut past: Vec<UsagePeriod> = Vec::new();
    for line in issued.iter().filter(|line| {
        !line.voided
            && touched.contains(&line.key.fee)
            && matches!(line.key.kind, ChargeKind::Usage { .. })
            && midnight(line.until) > change.at
    }) {
        let period = UsagePeriod {
            key: line.key.clone(),
        };
        if !past.contains(&period) {
            past.push(period);
        }
    }
    let within = |period: &UsagePeriod, need: &billing_core::evaluate::UsageNeed| {
        need.fee == period.key.fee
            && midnight(period.key.from) <= need.read.window.from
            && need.read.window.until <= midnight(period.key.until)
    };
    let needs: Vec<_> = evaluation
        .usage_needed
        .iter()
        .filter(|need| past.iter().any(|period| within(period, need)))
        .cloned()
        .collect();
    // With no window left to read, each period is read whole: one wholly past the ending owes
    // nothing, and its usage comes back.
    if needs.is_empty() {
        return Ok((evaluation, past));
    }
    let reads = super::usage::read_usage(client, loaded, &needs).await?;
    let evaluation =
        evaluate(&loaded.facts, change.at, Some(&reads), &StorePricer).map_err(pricing_error)?;
    let read = past
        .into_iter()
        .filter(|period| {
            !evaluation
                .usage_needed
                .iter()
                .any(|need| within(period, need))
        })
        .collect();
    Ok((evaluation, read))
}

/// `evaluation` with the usage a change ends that its day's recurring invoice, written before it,
/// did not bill: arrears of a fee off its own grid, which the change's adjustment bills (R-011).
async fn with_usage_riding_on_its_day(
    client: &dyn UsageClient,
    loaded: &super::load::LoadedFacts,
    evaluation: Evaluation,
    issued: &[IssuedLine],
    own: &[DocumentSlot],
    change: &Change,
) -> StoreResult<Evaluation> {
    let its_day = DocumentSlot::Recurring {
        period_start: change.on,
    };
    let billed_to_its_day = |fee: &Fee| {
        issued.iter().any(|line| {
            !line.voided
                && line.key.fee == *fee
                && matches!(line.key.kind, ChargeKind::Usage { .. })
                && line.until == change.on
        })
    };
    let needs: Vec<_> = evaluation
        .usage_needed
        .iter()
        .filter(|need| {
            need.document == its_day && own.contains(&its_day) && !billed_to_its_day(&need.fee)
        })
        .cloned()
        .collect();
    if needs.is_empty() {
        return Ok(evaluation);
    }
    let reads = super::usage::read_usage(client, loaded, &needs).await?;
    evaluate(&loaded.facts, change.at, Some(&reads), &StorePricer).map_err(pricing_error)
}

/// A change evaluated against what was issued.
struct Evaluated {
    facts: Facts<StoreFee>,
    evaluation: Evaluation,
    billed: Issued,
    own: Vec<DocumentSlot>,
    planned: Planned,
    on: NaiveDate,
}

/// `change` evaluated, `overlay` (the rows of a change not written yet) laid over the facts.
async fn evaluated(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    change: &Change,
    hypothesis: &Hypothesis,
    overlay: Option<&Overlay>,
    usage: Option<&dyn UsageClient>,
) -> StoreResult<Option<Evaluated>> {
    let mut loaded =
        load_facts_with(conn, tenant_id, subscription_id, change.on, None, overlay).await?;
    as_written(&mut loaded.facts, change.at);
    let bought = hypothesis.bought.as_ref();
    let touched = match bought {
        Some(bought) => {
            with_bought(&mut loaded.facts, bought, change.on);
            BTreeSet::from([bought.lineage.clone()])
        }
        None if !hypothesis.only.is_empty() => hypothesis.only.clone(),
        None => touched(&loaded.facts, change),
    };
    if touched.is_empty() {
        return Ok(None);
    }
    let mut evaluation =
        evaluate(&loaded.facts, change.at, None, &StorePricer).map_err(pricing_error)?;
    let from = evaluation
        .charges
        .iter()
        .filter(|charge| {
            touched.contains(&charge.key.fee)
                && (charge.key.until > change.on || charge.key.from >= change.on)
        })
        .map(|charge| charge.key.from)
        .fold(change.on, |a, b| a.min(b));
    let mut billed = load_billed(conn, tenant_id, subscription_id, from, None, None).await?;
    if bought.is_some() {
        let pending = pending_purchases(conn, tenant_id, subscription_id).await?;
        billed.lines.retain(|on| !pending.contains(&on.invoice));
    }
    let issued: Vec<IssuedLine> = billed.lines.iter().map(|on| on.line.clone()).collect();
    let own = own_documents(conn, tenant_id, subscription_id, change).await?;
    let mut read = Vec::new();
    match (usage, change.touched) {
        (Some(client), Touched::Ended) => {
            (evaluation, read) =
                with_usage_past_the_cutoff(client, &loaded, evaluation, &issued, &touched, change)
                    .await?;
        }
        (Some(client), Touched::Changed) => {
            evaluation =
                with_usage_riding_on_its_day(client, &loaded, evaluation, &issued, &own, change)
                    .await?;
        }
        (None, _) => {}
    }
    let drafts: BTreeSet<String> = billed
        .lines
        .iter()
        .filter(|on| on.draft)
        .map(|on| on.line.id.clone())
        .collect();
    let planned = plan(
        &evaluation,
        &issued,
        &touched,
        change,
        &own,
        loaded.facts.reconcile_from,
        &billed.noted,
        &read,
        &drafts,
        loaded.facts.ending.map(|ending| ending.on()),
    );
    for (key, amount) in &planned.short {
        log::warn!(
            "the change on {} of subscription {subscription_id} owes back {amount} of {:?} beyond what its lines have left",
            change.on,
            key.fee
        );
        if overlay.is_none() {
            record_short(tenant_id, change.label, *amount);
        }
    }
    Ok(Some(Evaluated {
        facts: loaded.facts,
        evaluation,
        billed,
        own,
        planned,
        on: change.on,
    }))
}

/// What a credit of `net` of a line reads as on a note: a share of its quantity when it was
/// billed by the unit, else an amount.
fn taken_back(line: &IssuedLine, net: i64) -> CreditLineItem {
    match line.quantity {
        Some(quantity) if line.unit_priced && quantity > Decimal::ZERO && line.subtotal > 0 => {
            CreditLineItem::Line {
                local_id: line.id.clone(),
                quantity: (Decimal::from(net) / Decimal::from(line.subtotal) * quantity)
                    .min(quantity),
            }
        }
        _ => CreditLineItem::Amount {
            local_id: line.id.clone(),
            amount_cents: net,
        },
    }
}

/// What the notes take back, each with its invoice, and its net total.
fn credits_of(
    evaluated: &Evaluated,
    give_back: GiveBack,
) -> (Vec<(InvoiceId, CreditLineItem)>, i64) {
    let mut credited = 0;
    let credits = evaluated
        .planned
        .credits
        .iter()
        .filter_map(|(id, amount)| {
            // A draft's line is credited once it is issued (R-078), never on these notes.
            let on = evaluated
                .billed
                .lines
                .iter()
                .find(|on| on.line.id == *id && !on.draft && !on.line.voided)?;
            let net = on.line.net_of(*amount);
            credited += net;
            let item = match give_back {
                GiveBack::LatestFirst => taken_back(&on.line, net),
                GiveBack::Unused => CreditLineItem::Amount {
                    local_id: id.clone(),
                    amount_cents: net,
                },
            };
            Some((on.invoice, item))
        })
        .collect();
    (credits, credited)
}

/// A change's line as adjustment invoices show it: a stub "(prorated)", a whole period plain, a one-time fee
/// "(one-time)", what a new price adds to a fee billed before "(adjustment)"; by the unit for a
/// pack, a one-time fee and a new add-on bought several times, else one of its amount.
fn as_change_line(
    mut line: LineItem,
    row: &FeeFact<StoreFee>,
    charge: &Charge,
    amount: i64,
    repriced: bool,
) -> LineItem {
    let fee = &row.fee;
    let precision = u32::from(fee.precision);
    // A unit price to 8 places, half-up.
    let per = |amount: i64, quantity: Decimal| {
        (Decimal::new(amount, precision) / quantity)
            .round_dp_with_strategy(8, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
    };
    let pack = charge.key.kind == ChargeKind::Credit;
    line.local_id = uuid::Uuid::now_v7().to_string();
    line.description = None;
    match row.billing {
        Billing::Arrears => {}
        Billing::OneTime if !pack => {
            line.name = format!("{} (one-time)", fee.name);
            line.is_prorated = false;
        }
        _ if pack => {
            let prorated = charge.share.is_some();
            line.name = if prorated {
                format!("{} (prorated)", fee.name)
            } else {
                fee.name.clone()
            };
            line.is_prorated = prorated;
            if let Some(quantity) = line.quantity.filter(|q| !q.is_zero()) {
                line.unit_price = Some(per(amount.max(0), quantity));
            }
            if !is_period_tied(&fee.fee) {
                line.end_date = line.start_date;
            }
        }
        _ => {
            // A stub bills part of its period; a line billing it whole is no stub.
            let whole = !repriced && charge.share.is_none();
            line.name = if repriced {
                format!("{} (adjustment)", fee.name)
            } else if whole {
                fee.name.clone()
            } else {
                format!("{} (prorated)", fee.name)
            };
            line.is_prorated = !whole;
            let quantity = match fee.sub_add_on_id {
                Some(_) if fee.instances > Decimal::ONE && !repriced => fee.instances,
                _ => Decimal::ONE,
            };
            line.quantity = Some(quantity);
            line.unit_price = Some(per(amount, quantity));
        }
    }
    line
}

/// A seat purchase's line as its invoice shows it: the seats bought, each at their share of what
/// they bill to 8 places, half-up. Its id is random and it names its slot fee, so reconcile reads
/// it on that fee's charge for the period (R-075).
fn as_seat_line(
    mut line: LineItem,
    row: &FeeFact<StoreFee>,
    bought: &Bought,
    amount: i64,
) -> LineItem {
    let fee = &row.fee;
    let unit = match &fee.fee {
        crate::domain::SubscriptionFee::Slot { unit, .. } => unit.as_str(),
        _ => fee.name.as_str(),
    };
    let seats = Decimal::from(bought.seats);
    line.local_id = crate::utils::local_id::LocalId::no_prefix();
    line.name = format!("{} {unit} upgrade", bought.seats);
    line.description = Some(format!(
        "Prorated charge for {} additional {unit}",
        bought.seats
    ));
    line.quantity = Some(seats);
    line.unit_price = (!seats.is_zero()).then(|| {
        (Decimal::new(amount, u32::from(fee.precision)) / seats)
            .round_dp_with_strategy(8, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
    });
    line.is_prorated = true;
    line
}

/// The lines of what `evaluated` bills; those of the fee whose seats are `bought`, as the
/// purchase's.
async fn bills_of(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    evaluated: &Evaluated,
    bought: Option<&Bought>,
) -> StoreResult<Vec<LineItem>> {
    let lineages = lineages_of(&evaluated.facts);
    let no_usage = HashMap::new();
    let mut lines = Vec::new();
    for (key, amount) in &evaluated.planned.bills {
        let charges: Vec<&Charge> = evaluated
            .evaluation
            .charges
            .iter()
            .filter(|c| c.key == *key && evaluated.own.contains(&c.document))
            .collect();
        let Some(first) = charges.first().copied() else {
            continue;
        };
        let Some(row) = row_of(&evaluated.facts, first) else {
            continue;
        };
        // A product's `once` credit is billed once per subscription (R16, OQ6).
        if let Some(product) = first.once_per_product
            && InvoiceRow::subscription_billed_once_credit(
                conn,
                tenant_id,
                subscription_id,
                product,
                None,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        {
            continue;
        }
        let repriced = evaluated
            .evaluation
            .charges
            .iter()
            .any(|c| c.key == *key && !evaluated.own.contains(&c.document))
            || evaluated
                .billed
                .lines
                .iter()
                .any(|on| on.line.from < evaluated.on && on.line.bills(key));
        let line = render(
            &charges,
            *amount,
            row,
            row.fee.precision,
            subscription_id,
            &no_usage,
            &lineages,
        )?;
        lines.push(match bought.filter(|bought| bought.lineage == key.fee) {
            Some(bought) => as_seat_line(line, row, bought, *amount),
            None => as_change_line(line, row, first, *amount, repriced),
        });
    }
    // As adjustment invoices list them: components, then add-ons, packs last.
    lines.sort_by_key(|line| {
        (
            line.kind == crate::domain::LineItemKind::Credit,
            line.sub_add_on_id.is_some(),
        )
    });
    Ok(lines)
}

/// What `change` owes and owes back, from the facts written by now, `overlay` laid over them for
/// a change not written yet: a change and its preview share this computation.
pub(in crate::services) async fn change_documents(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    change: &Change,
    overlay: Option<&Overlay>,
    usage: Option<&dyn UsageClient>,
) -> StoreResult<ChangeDocuments> {
    documents_of(
        conn,
        tenant_id,
        subscription_id,
        change,
        &Hypothesis::default(),
        overlay,
        usage,
    )
    .await
}

async fn documents_of(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    change: &Change,
    hypothesis: &Hypothesis,
    overlay: Option<&Overlay>,
    usage: Option<&dyn UsageClient>,
) -> StoreResult<ChangeDocuments> {
    let Some(evaluated) = evaluated(
        conn,
        tenant_id,
        subscription_id,
        change,
        hypothesis,
        overlay,
        usage,
    )
    .await?
    else {
        return Ok(ChangeDocuments::default());
    };
    let bills = bills_of(
        conn,
        tenant_id,
        subscription_id,
        &evaluated,
        hypothesis.bought.as_ref(),
    )
    .await?;
    let (credits, credited) = credits_of(&evaluated, change.give_back);
    // What the lines bill: a charge rendered as no line (a `once` credit billed) bills nothing.
    let charged = bills.iter().map(|line| line.amount_subtotal).sum();
    Ok(ChangeDocuments {
        bills,
        credits,
        charged,
        credited,
    })
}

/// What `change` owes back, from the facts written by now; an ending's usage read through
/// `usage`.
pub(in crate::services) async fn change_credits(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    change: &Change,
    usage: &dyn UsageClient,
) -> StoreResult<Vec<(InvoiceId, CreditLineItem)>> {
    Ok(evaluated(
        conn,
        tenant_id,
        subscription_id,
        change,
        &Hypothesis::default(),
        None,
        Some(usage),
    )
    .await?
    .map(|evaluated| credits_of(&evaluated, change.give_back).0)
    .unwrap_or_default())
}

/// What seats bought on a day bill: the slot fee's charge for its period from the facts with
/// them held, less what its lines billed (R-079, R-080), and the days the charge covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::services) struct SeatPurchase {
    pub amount: i64,
    pub from: NaiveDate,
    pub until: NaiveDate,
}

/// A seat purchase and the documents it writes: its adjustment invoice's lines, no note.
#[derive(Debug)]
pub(in crate::services) struct SeatDocuments {
    pub purchase: SeatPurchase,
    pub documents: ChangeDocuments,
}

/// What buying `bought` on `on` owes on that day's documents, and the adjustment's lines billing
/// it; None when no charge of its fee is due there (a free trial, a period its renewal still has
/// to bill). Seats bought only add to what their fee owes: nothing is taken back.
pub(in crate::services) async fn seat_purchase(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    on: NaiveDate,
    bought: &Bought,
) -> StoreResult<Option<SeatDocuments>> {
    let change = Change::of_rows(on, GiveBack::LatestFirst, "seats");
    let hypothesis = Hypothesis {
        bought: Some(bought.clone()),
        ..Hypothesis::default()
    };
    let Some(evaluated) = evaluated(
        conn,
        tenant_id,
        subscription_id,
        &change,
        &hypothesis,
        None,
        None,
    )
    .await?
    else {
        return Ok(None);
    };
    let Some(purchase) = purchase_of(
        &evaluated.evaluation,
        &evaluated.own,
        &evaluated.planned,
        &bought.lineage,
    ) else {
        return Ok(None);
    };
    let bills = bills_of(conn, tenant_id, subscription_id, &evaluated, Some(bought)).await?;
    let charged = bills.iter().map(|line| line.amount_subtotal).sum();
    Ok(Some(SeatDocuments {
        purchase,
        documents: ChangeDocuments {
            bills,
            credits: Vec::new(),
            charged,
            credited: 0,
        },
    }))
}

/// The period of `lineage` on its own grid holding `on`, as the facts bill it: a run carried
/// through a change of the subscription's cadence keeps its own (R-084). None when no charge of
/// the fee holds `on` (a free trial).
pub(in crate::services) async fn fee_period_on(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    on: NaiveDate,
    lineage: &Fee,
) -> StoreResult<Option<(NaiveDate, NaiveDate)>> {
    let mut loaded = load_facts_with(conn, tenant_id, subscription_id, on, None, None).await?;
    as_written(&mut loaded.facts, midnight(on));
    let evaluation =
        evaluate(&loaded.facts, midnight(on), None, &StorePricer).map_err(pricing_error)?;
    Ok(evaluation
        .charges
        .iter()
        .filter(|charge| {
            charge.key.fee == *lineage
                && charge.key.kind == ChargeKind::Fee
                && charge.key.from <= on
                && on < charge.key.until
        })
        .map(|charge| (charge.key.from, charge.key.until))
        .max())
}

/// What the add-ons `overlay` opens bill on its day's documents: the stubs of their own fees from
/// the facts with their rows laid over (R-079, R-080), in the figures they are priced in, neither
/// taxed nor covered. The rows are not written yet.
pub(in crate::services) async fn add_on_purchase(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    overlay: &Overlay,
) -> StoreResult<ChangeDocuments> {
    let change = Change::of_rows(overlay.rows.on, GiveBack::LatestFirst, "add-on purchase");
    let hypothesis = Hypothesis {
        bought: None,
        only: overlay
            .rows
            .opens_add_ons
            .iter()
            .map(|row| Fee::AddOn(row.lineage_id.unwrap_or(row.id)))
            .collect(),
    };
    documents_of(
        conn,
        tenant_id,
        subscription_id,
        &change,
        &hypothesis,
        Some(overlay),
        None,
    )
    .await
}

fn purchase_of(
    evaluation: &Evaluation,
    own: &[DocumentSlot],
    planned: &Planned,
    lineage: &Fee,
) -> Option<SeatPurchase> {
    let bills = |key: &ChargeKey| key.fee == *lineage && key.kind == ChargeKind::Fee;
    let windows: Vec<(NaiveDate, NaiveDate)> = evaluation
        .charges
        .iter()
        .filter(|charge| bills(&charge.key) && own.contains(&charge.document))
        .map(|charge| charge.window)
        .collect();
    let from = windows.iter().map(|window| window.0).min()?;
    let until = windows.iter().map(|window| window.1).max()?;
    let amount = planned
        .bills
        .iter()
        .filter(|(key, _)| bills(key))
        .map(|(_, amount)| *amount)
        .sum();
    Some(SeatPurchase {
        amount,
        from,
        until,
    })
}

/// The finalized invoices among `ids` as notes against them read them; a draft merged into a
/// consolidated invoice is read as its parent, only this subscription's lines.
pub(in crate::services) async fn credit_context_of(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    ids: &[InvoiceId],
) -> StoreResult<PeriodCreditContext> {
    let mut ctx = PeriodCreditContext {
        invoices: Vec::new(),
        all_lines: Vec::new(),
        line_to_invoice: HashMap::new(),
        line_lineage: HashMap::new(),
        already_credited: HashMap::new(),
        corrected: HashMap::new(),
        returned: HashMap::new(),
    };
    for id in ids {
        let mut row = InvoiceRow::find_detailed_by_id(conn, tenant_id, *id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .invoice;
        if let Some(parent) = row.consolidated_into_invoice_id {
            row = InvoiceRow::find_detailed_by_id(conn, tenant_id, parent)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .invoice;
        }
        let mut invoice: Invoice = row.try_into()?;
        if invoice.status != InvoiceStatusEnum::Finalized
            || ctx.invoices.iter().any(|seen| seen.id == invoice.id)
        {
            continue;
        }
        if invoice.subscription_id.is_none() {
            let owners = crate::services::credit::scope::LineOwners::load_tx(
                conn,
                tenant_id,
                &invoice.line_items,
            )
            .await?;
            invoice.line_items.retain(|line| {
                crate::services::credit::scope::line_subscription(line, None, &owners)
                    == Some(subscription_id)
            });
        }
        let (line_lineage, already_credited, corrected, returned) =
            load_credit_matching_context(conn, tenant_id, &invoice).await?;
        ctx.line_lineage.extend(line_lineage);
        ctx.already_credited.extend(already_credited);
        ctx.corrected.extend(corrected);
        ctx.returned.extend(returned);
        for line in &invoice.line_items {
            ctx.line_to_invoice
                .insert(line.local_id.clone(), invoice.id);
            ctx.all_lines.push(line.clone());
        }
        ctx.invoices.push(invoice);
    }
    Ok(ctx)
}

/// An adjustment invoice's frame for lines dated `on`: the period it sits in.
pub(in crate::services) fn adjustment_dated(
    subscription: &Subscription,
    on: NaiveDate,
) -> ProrationResult {
    ProrationResult {
        lines: Vec::new(),
        net_amount_cents: 0,
        change_date: on,
        period_start: subscription.current_period_start,
        period_end: subscription.current_period_end.unwrap_or(on),
        proration_factor: 0.0,
    }
}

impl Services {
    /// The adjustment invoice `bills` make, dated `on`, taxed and its credit estimated, unwritten.
    pub(in crate::services) async fn change_adjustment_content(
        &self,
        conn: &mut PgConn,
        subscription: &Subscription,
        customer: &Customer,
        on: NaiveDate,
        bills: &[LineItem],
    ) -> StoreResult<AdjustmentInvoiceContent> {
        self.compute_adjustment_invoice_content(
            conn,
            subscription.tenant_id,
            subscription,
            customer,
            &adjustment_dated(subscription, on),
            bills,
        )
        .await
    }

    /// Writes and finalizes the adjustment invoice `bills` make, dated `on`; none without lines.
    pub(in crate::services) async fn issue_change_adjustment_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        subscription: &Subscription,
        customer: &Customer,
        on: NaiveDate,
        bills: &[LineItem],
    ) -> StoreResult<Option<Invoice>> {
        if bills.is_empty() {
            return Ok(None);
        }
        let invoice = self
            .create_adjustment_invoice(
                conn,
                subscription.tenant_id,
                subscription,
                customer,
                &adjustment_dated(subscription, on),
                bills,
            )
            .await?;
        if let Some(invoice) = &invoice {
            self.finalize_invoice_tx(
                conn,
                actor,
                invoice.id,
                subscription.tenant_id,
                false,
                &None,
            )
            .await?;
        }
        Ok(invoice)
    }

    /// Settles `change`, its facts written: what it owes back on notes against the finalized
    /// invoices that billed it, dated at its instant, then what it owes on a finalized adjustment
    /// invoice, which what the notes return to the balance pays. Returns the adjustment invoice
    /// and the notes.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn settle_change_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        subscription: &Subscription,
        customer: &Customer,
        change: &Change,
        reason: &str,
        origin: CreditNoteOrigin,
    ) -> StoreResult<(Option<InvoiceId>, Vec<common_domain::ids::CreditNoteId>)> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let documents = change_documents(
                conn,
                subscription.tenant_id,
                subscription.id,
                change,
                None,
                Some(self.usage_client.as_ref()),
            )
            .await?;
            let mut invoices: Vec<InvoiceId> =
                documents.credits.iter().map(|(id, _)| *id).collect();
            invoices.sort_by_key(|id| id.as_uuid());
            invoices.dedup();
            let ctx =
                credit_context_of(conn, subscription.tenant_id, subscription.id, &invoices).await?;
            let notes = self
                .issue_period_credit_notes_tx(
                    conn,
                    actor,
                    subscription.tenant_id,
                    &ctx,
                    documents
                        .credits
                        .into_iter()
                        .map(|(_, item)| item)
                        .collect(),
                    reason,
                    false,
                    Some(change.at),
                    origin,
                )
                .await?;
            let invoice = self
                .issue_change_adjustment_tx(
                    conn,
                    actor,
                    subscription,
                    customer,
                    change.on,
                    &documents.bills,
                )
                .await?;
            Ok((
                invoice.map(|invoice| invoice.id),
                notes.into_iter().map(|note| note.id).collect(),
            ))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use billing_core::charge::Denomination;
    use billing_core::facts::{Ending, EndingKind, PlanWindow, Start};
    use billing_core::issued::IssuedKind;
    use common_domain::ids::{PlanVersionId, SubscriptionPriceComponentId};
    use std::collections::BTreeMap;

    use crate::domain::SubscriptionFee;
    use crate::domain::enums::SubscriptionFeeBillingPeriod;
    use crate::services::evaluate::pricer::UnitConversion;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, m, day).unwrap()
    }

    fn subscription() -> SubscriptionId {
        SubscriptionId::from(uuid::Uuid::from_u128(1))
    }

    fn lineage(n: u128) -> Fee {
        Fee::Component(SubscriptionPriceComponentId::from(uuid::Uuid::from_u128(n)))
    }

    /// A monthly fee of `rate` (in units of 1.00) of lineage `n`, in force over `[from, to)`.
    fn row(
        n: u128,
        id: u128,
        rate: i64,
        from: NaiveDate,
        to: Option<NaiveDate>,
    ) -> FeeFact<StoreFee> {
        FeeFact {
            id: uuid::Uuid::from_u128(id),
            lineage: lineage(n),
            from,
            to,
            billing: Billing::Advance,
            cadence: 1,
            plan_component: true,
            fee: StoreFee {
                fee: SubscriptionFee::Rate {
                    rate: Decimal::from(rate),
                },
                period: SubscriptionFeeBillingPeriod::Monthly,
                name: format!("Fee {n}"),
                product_id: None,
                price_component_id: None,
                sub_component_id: Some(SubscriptionPriceComponentId::from(uuid::Uuid::from_u128(
                    id,
                ))),
                sub_add_on_id: None,
                currency: "EUR".to_owned(),
                precision: 2,
                instances: Decimal::ONE,
                conversion: UnitConversion::default(),
                pinned: BTreeMap::new(),
            },
        }
    }

    fn facts(
        start: NaiveDate,
        fees: Vec<FeeFact<StoreFee>>,
        ending: Option<Ending>,
    ) -> Facts<StoreFee> {
        Facts {
            subscription: subscription(),
            billing_day_anchor: 1,
            start: Start {
                date: start,
                settled: Some(midnight(start)),
                checkout: false,
            },
            trial: None,
            plan_windows: vec![PlanWindow {
                plan_version: PlanVersionId::new(),
                from: start,
                to: None,
                cadence: 1,
                cadence_origin: start,
            }],
            fees,
            seats: vec![],
            coupons: vec![],
            minimums: vec![],
            ending,
            end_date: None,
            reconcile_from: None,
            arrears_from: None,
            ending_recorded: None,
            ending_day_invoiced: false,
        }
    }

    /// An issued line of lineage `n` over `[from, until)` billing `subtotal`.
    fn issued(id: &str, n: u128, from: NaiveDate, until: NaiveDate, subtotal: i64) -> IssuedLine {
        IssuedLine {
            id: id.to_owned(),
            key: ChargeKey {
                subscription: Some(subscription()),
                fee: lineage(n),
                from,
                until,
                kind: ChargeKind::Fee,
                denomination: Denomination::Money,
            },
            kind: IssuedKind::Fee {
                price_component: None,
                add_on: false,
            },
            lineage: None,
            quantity: Some(Decimal::ONE),
            unit_priced: true,
            from,
            until,
            subtotal,
            quoted: None,
            already_credited: 0,
            corrected: 0,
            settled: None,
            voided: false,
        }
    }

    fn planned(facts: &Facts<StoreFee>, lines: &[IssuedLine], change: &Change) -> Planned {
        let evaluation = evaluate(facts, change.at, None, &StorePricer).unwrap();
        let own = [DocumentSlot::Adjustment { on: change.on }];
        let own: &[DocumentSlot] = if change.touched == Touched::Ended {
            &[]
        } else {
            &own
        };
        plan(
            &evaluation,
            lines,
            &touched(facts, change),
            change,
            own,
            facts.reconcile_from,
            &HashMap::new(),
            &[],
            &BTreeSet::new(),
            facts.ending.map(|ending| ending.on()),
        )
    }

    fn key(n: u128, from: NaiveDate, until: NaiveDate) -> ChargeKey {
        ChargeKey {
            subscription: Some(subscription()),
            fee: lineage(n),
            from,
            until,
            kind: ChargeKind::Fee,
            denomination: Denomination::Money,
        }
    }

    /// Starter's 29.00 billed for January, Pro's 99.00 from January 16 (16 of 31 days left).
    /// Starter gives back 2900 × 16/31 = 1496.77 → 1497 from its line, by unused time;
    /// Pro bills 9900 × 16/31 = 5109.68 → 5110 on the adjustment.
    /// Rules: R-077, R-079, R-080
    #[test]
    fn a_plan_change_bills_the_new_stub_and_gives_back_the_old_unused_time() {
        let facts = facts(
            d(1, 1),
            vec![
                row(1, 11, 29, d(1, 1), Some(d(1, 16))),
                row(2, 21, 99, d(1, 16), None),
            ],
            None,
        );
        let lines = [issued("starter", 1, d(1, 1), d(2, 1), 2_900)];
        let change = Change::of_rows(d(1, 16), GiveBack::Unused, "plan change");
        assert_eq!(
            planned(&facts, &lines, &change),
            Planned {
                bills: vec![(key(2, d(1, 1), d(2, 1)), 5_110)],
                credits: vec![("starter".to_owned(), 1_497)],
                short: vec![],
            }
        );
    }

    /// 31.00 billed for January, lowered to 15.00 from January 11 (21 of 31 days left): what is
    /// left of each price, 3100 × 21/31 = 2100 and 1500 × 21/31 = 1016.13 → 1016, differs by
    /// 1084, taken back from the latest line; nothing billed.
    /// Rules: R-078, R-079, R-080
    #[test]
    fn a_lower_price_takes_back_what_is_left_of_the_difference() {
        let facts = facts(
            d(1, 1),
            vec![
                row(1, 11, 31, d(1, 1), Some(d(1, 11))),
                row(1, 12, 15, d(1, 11), None),
            ],
            None,
        );
        let lines = [issued("january", 1, d(1, 1), d(2, 1), 3_100)];
        let change = Change::of_rows(d(1, 11), GiveBack::LatestFirst, "amendment");
        assert_eq!(
            planned(&facts, &lines, &change),
            Planned {
                bills: vec![],
                credits: vec![("january".to_owned(), 1_084)],
                short: vec![],
            }
        );
    }

    /// 15.00 billed for January, raised to 31.00 from January 11: 3100 × 21/31 = 2100 left of
    /// the new price, 1500 × 21/31 = 1016.13 → 1016 of the old, 1084 billed; nothing taken back.
    /// Rules: R-079, R-080
    #[test]
    fn a_higher_price_bills_what_is_left_of_the_difference() {
        let facts = facts(
            d(1, 1),
            vec![
                row(1, 11, 15, d(1, 1), Some(d(1, 11))),
                row(1, 12, 31, d(1, 11), None),
            ],
            None,
        );
        let lines = [issued("january", 1, d(1, 1), d(2, 1), 1_500)];
        let change = Change::of_rows(d(1, 11), GiveBack::LatestFirst, "amendment");
        assert_eq!(
            planned(&facts, &lines, &change),
            Planned {
                bills: vec![(key(1, d(1, 1), d(2, 1)), 1_084)],
                credits: vec![],
                short: vec![],
            }
        );
    }

    /// 310.00 billed for October, raised to 620.00 from the 11th and billed 420.00 more for its
    /// 21 days; cancelled on the 17th, 15 of 31 days left. Owed: 310.00's first ten days,
    /// 31000 − 31000 × 21/31 = 10000, and 620.00's next six, 62000 × 21/31 − 62000 × 15/31 =
    /// 42000 − 30000 = 12000: 22000 of the 52000 billed. Each line gives back its unused time:
    /// 31000 × 15/31 = 15000 and 21000 × 15/21 = 15000.
    /// Rules: R-077, R-080
    #[test]
    fn an_ending_takes_back_each_line_by_its_unused_time() {
        let facts = facts(
            d(10, 1),
            vec![
                row(1, 11, 310, d(10, 1), Some(d(10, 11))),
                row(1, 12, 620, d(10, 11), None),
            ],
            Some(Ending {
                at: midnight(d(10, 17)),
                kind: EndingKind::Cancel,
            }),
        );
        let lines = [
            issued("first", 1, d(10, 1), d(11, 1), 31_000),
            issued("raised", 1, d(10, 11), d(11, 1), 21_000),
        ];
        let change = Change::of_ending(d(10, 17), midnight(d(10, 17)));
        let mut credits = planned(&facts, &lines, &change).credits;
        credits.sort();
        assert_eq!(
            credits,
            vec![("first".to_owned(), 15_000), ("raised".to_owned(), 15_000)]
        );
    }

    /// Cancelled on February 1 after February's 29.00 was billed: no day of it is used, the
    /// whole line comes back.
    /// Rules: R-077
    #[test]
    fn an_ending_on_the_first_day_gives_the_whole_period_back() {
        let facts = facts(
            d(1, 1),
            vec![row(1, 11, 29, d(1, 1), None)],
            Some(Ending {
                at: midnight(d(2, 1)),
                kind: EndingKind::Cancel,
            }),
        );
        let lines = [
            issued("january", 1, d(1, 1), d(2, 1), 2_900),
            issued("february", 1, d(2, 1), d(3, 1), 2_900),
        ];
        let change = Change::of_ending(d(2, 1), midnight(d(2, 1)));
        assert_eq!(
            planned(&facts, &lines, &change).credits,
            vec![("february".to_owned(), 2_900)]
        );
    }

    /// A fee no row touches on the change's day is left alone, though its line differs from
    /// what the facts owe.
    #[test]
    fn a_fee_the_change_does_not_touch_is_left_alone() {
        let facts = facts(
            d(1, 1),
            vec![
                row(1, 11, 29, d(1, 1), None),
                row(2, 21, 10, d(1, 16), None),
            ],
            None,
        );
        let lines = [issued("other", 1, d(1, 1), d(2, 1), 5_000)];
        let change = Change::of_rows(d(1, 16), GiveBack::LatestFirst, "amendment");
        // 1000 × 16/31 = 516.13 → 516 for the add-on; lineage 1 untouched.
        assert_eq!(
            planned(&facts, &lines, &change),
            Planned {
                bills: vec![(key(2, d(1, 1), d(2, 1)), 516)],
                credits: vec![],
                short: vec![],
            }
        );
    }

    /// A slot fee of `rate` a seat (in units of 1.00), in force from `from`.
    fn slot_row(n: u128, rate: i64, from: NaiveDate) -> FeeFact<StoreFee> {
        let mut slot = row(n, n * 10, rate, from, None);
        slot.fee.fee = SubscriptionFee::Slot {
            unit: "Seats".to_owned(),
            unit_rate: Decimal::from(rate),
            min_slots: None,
            max_slots: None,
            initial_slots: 1,
        };
        slot
    }

    fn seats(n: u128, at: NaiveDate, quantity: u64) -> SeatStep {
        SeatStep {
            id: SlotTransactionId::from(uuid::Uuid::from_u128(100 + n)),
            lineage: lineage(n),
            at,
            quantity,
            removed_at_renewal: 0,
        }
    }

    /// What buying `seats` of lineage `n` on `on` bills, `own` the change's documents.
    fn bought(
        mut facts: Facts<StoreFee>,
        lines: &[IssuedLine],
        n: u128,
        added: u64,
        on: NaiveDate,
        own: &[DocumentSlot],
    ) -> Option<SeatPurchase> {
        let bought = Bought {
            lineage: lineage(n),
            seats: added,
        };
        with_bought(&mut facts, &bought, on);
        let change = Change::of_rows(on, GiveBack::LatestFirst, "seats");
        let evaluation = evaluate(&facts, change.at, None, &StorePricer).unwrap();
        let touched = BTreeSet::from([bought.lineage.clone()]);
        let planned = plan(
            &evaluation,
            lines,
            &touched,
            &change,
            own,
            None,
            &HashMap::new(),
            &[],
            &BTreeSet::new(),
            None,
        );
        purchase_of(&evaluation, own, &planned, &bought.lineage)
    }

    /// A fee of 31.00 a month ended on Jan 20 by a plan change, billed by a finalized line of
    /// 3100 and a draft's line of 1000 from Jan 10: what the change gives back is split between
    /// the two, so while the draft waits for its issue the finalized line waits with it. A share
    /// taken from the finalized line alone was split again at every pass, each one issuing a
    /// note more.
    /// Rules: R-077, R-078
    #[test]
    fn a_give_back_split_with_a_drafts_line_waits_for_the_draft() {
        let facts = facts(d(1, 1), vec![row(1, 10, 31, d(1, 1), Some(d(1, 20)))], None);
        let change = Change::of_rows(d(1, 20), GiveBack::Unused, "plan change");
        let evaluation = evaluate(&facts, change.at, None, &StorePricer).unwrap();
        let touched = BTreeSet::from([lineage(1)]);
        let mut january = issued("january", 1, d(1, 1), d(2, 1), 3100);
        let extra = issued("draft", 1, d(1, 10), d(2, 1), 1000);
        let plan_with = |january: &IssuedLine, drafts: &BTreeSet<String>| {
            plan(
                &evaluation,
                &[january.clone(), extra.clone()],
                &touched,
                &change,
                &[],
                None,
                &HashMap::new(),
                &[],
                drafts,
                None,
            )
            .credits
        };
        let issued_all = plan_with(&january, &BTreeSet::new());
        assert_eq!(
            issued_all.iter().map(|(_, amount)| amount).sum::<i64>(),
            2200
        );
        let drafts = BTreeSet::from(["draft".to_owned()]);
        assert_eq!(plan_with(&january, &drafts), vec![]);
        // As if the finalized line's share had been issued: nothing more is ever split onto it.
        let share = issued_all
            .iter()
            .find(|(line, _)| line == "january")
            .map(|(_, amount)| *amount)
            .expect("a share of January");
        january.corrected = share;
        january.already_credited = share;
        assert_eq!(plan_with(&january, &drafts), vec![]);
    }

    /// One 10.00 seat to a term ending Feb 15, 2024, two more from Feb 8, of February's 29 days:
    /// three seats bill 3000 × 22/29 (2275.86 → 2276) − 3000 × 15/29 (1551.72 → 1552) = 724 of
    /// the week, the one held 1000 × 22/29 (758.62 → 759) − 1000 × 15/29 (517.24 → 517) = 242:
    /// 482 bought. Rounding the two seats' share once instead, 2000 × 22/29 (1517.24 → 1517) −
    /// 2000 × 15/29 (1034.48 → 1034), would bill 483 (R-079).
    /// Rules: R-079, R-080, R-084
    #[test]
    fn seats_bought_bill_the_difference_of_the_rounded_totals() {
        let mut facts = facts(d(1, 1), vec![slot_row(1, 10, d(1, 1))], None);
        facts.end_date = Some(d(2, 15));
        facts.seats = vec![seats(1, d(1, 1), 1)];
        let lines = [
            issued("january", 1, d(1, 1), d(2, 1), 1_000),
            issued("february", 1, d(2, 1), d(2, 15), 483),
        ];
        assert_eq!(
            bought(
                facts,
                &lines,
                1,
                2,
                d(2, 8),
                &[DocumentSlot::Adjustment { on: d(2, 8) }]
            ),
            Some(SeatPurchase {
                amount: 482,
                from: d(2, 8),
                until: d(2, 15),
            })
        );
    }

    /// Bought on a period's first day after its invoice billed one seat: the period at three
    /// seats, 3000, less the 1000 billed, whole.
    /// Rules: R-080
    #[test]
    fn seats_bought_on_a_renewal_day_bill_the_whole_period() {
        let mut facts = facts(d(1, 1), vec![slot_row(1, 10, d(1, 1))], None);
        facts.seats = vec![seats(1, d(1, 1), 1)];
        let lines = [
            issued("january", 1, d(1, 1), d(2, 1), 1_000),
            issued("february", 1, d(2, 1), d(3, 1), 1_000),
        ];
        assert_eq!(
            bought(
                facts,
                &lines,
                1,
                2,
                d(2, 1),
                &[
                    DocumentSlot::Adjustment { on: d(2, 1) },
                    DocumentSlot::Recurring {
                        period_start: d(2, 1)
                    },
                ]
            ),
            Some(SeatPurchase {
                amount: 2_000,
                from: d(2, 1),
                until: d(3, 1),
            })
        );
    }

    /// Seats bought the day others were: the day's last count stands, the purchase on top of it.
    #[test]
    fn seats_bought_add_to_those_changed_the_same_day() {
        let mut facts = facts(d(1, 1), vec![slot_row(1, 10, d(1, 1))], None);
        facts.seats = vec![seats(1, d(1, 1), 1), seats(2, d(1, 16), 3)];
        facts.seats[1].lineage = lineage(1);
        with_bought(
            &mut facts,
            &Bought {
                lineage: lineage(1),
                seats: 2,
            },
            d(1, 16),
        );
        let on_the_day: Vec<u64> = facts
            .seats
            .iter()
            .filter(|step| step.at == d(1, 16))
            .map(|step| step.quantity)
            .collect();
        assert_eq!(on_the_day, vec![5]);
    }

    /// A row cut by a dated price on the change's day is no change of that fee.
    /// Rules: R-097
    #[test]
    fn a_dated_price_is_not_a_change() {
        let facts = facts(
            d(1, 1),
            vec![
                row(1, 11, 29, d(1, 1), Some(d(1, 16))),
                row(1, 11, 39, d(1, 16), None),
            ],
            None,
        );
        let change = Change::of_rows(d(1, 16), GiveBack::LatestFirst, "amendment");
        assert!(touched(&facts, &change).is_empty());
    }

    /// 1.00 a month billed for January before the watermark on Feb 1, amended on its lineage to
    /// 0.00 a year from Feb 1: January's period is settled, so its line is no part of the year's
    /// charge that holds it, and nothing is given back.
    /// Rules: R-094
    #[test]
    fn a_line_of_a_settled_period_is_never_taken_back_by_a_longer_period_holding_it() {
        let mut yearly = row(1, 12, 0, d(2, 1), None);
        yearly.cadence = 12;
        yearly.fee.period = SubscriptionFeeBillingPeriod::Annual;
        let mut facts = facts(
            d(1, 1),
            vec![row(1, 11, 1, d(1, 1), Some(d(2, 1))), yearly],
            None,
        );
        facts.reconcile_from = Some(d(2, 1));
        let lines = [issued("january", 1, d(1, 1), d(2, 1), 100)];
        let change = Change::of_rows(d(2, 1), GiveBack::LatestFirst, "amendment");
        assert_eq!(planned(&facts, &lines, &change), Planned::default());
    }

    /// 91.00 a quarter from Jan 1, 60.00 billed on a January line before the watermark on Feb 1,
    /// lowered to 0.00 from Feb 15: no January period of the fee was settled, so January's line is
    /// the quarter's. It owes 9100 − 9100 × 46/91 = 4500, so 1500 comes back from the line.
    /// Rules: R-094
    #[test]
    fn a_line_whose_days_a_longer_charge_bills_is_that_charges_after_the_watermark() {
        let quarterly = |id: u128, rate: i64, from: NaiveDate, to: Option<NaiveDate>| {
            let mut row = row(1, id, rate, from, to);
            row.cadence = 3;
            row.fee.period = SubscriptionFeeBillingPeriod::Quarterly;
            row
        };
        let mut facts = facts(
            d(1, 1),
            vec![
                quarterly(11, 91, d(1, 1), Some(d(2, 15))),
                quarterly(12, 0, d(2, 15), None),
            ],
            None,
        );
        facts.reconcile_from = Some(d(2, 1));
        let lines = [issued("january", 1, d(1, 1), d(2, 1), 6_000)];
        let change = Change::of_rows(d(2, 15), GiveBack::LatestFirst, "amendment");
        assert_eq!(
            planned(&facts, &lines, &change),
            Planned {
                bills: vec![],
                credits: vec![("january".to_owned(), 1_500)],
                short: vec![],
            }
        );
    }
}
