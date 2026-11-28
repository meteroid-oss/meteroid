//! The derived state, written from the facts by one writer: the status, the period, the plan
//! version and its cadence, `pending_checkout` and the MRR; each transition's logs and outbox,
//! stamped at the instant its fact put it; the documents now due; and when to look again.

use std::collections::HashMap;

use billing_core::evaluate::{DocumentSlot, Status, evaluate};
use billing_core::facts::{Ending, EndingKind, Facts};
use billing_core::transition::{Deadlines, Due, Step, Stored, Transition, step};
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CreditNoteId, PlanVersionId, SubscriptionId, TenantId};
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::enums::{
    InvoiceStatusEnum, InvoiceType as InvoiceKind, PlanTypeEnum, SubscriptionChangeKindEnum,
    SubscriptionEventType,
};
use diesel_models::invoices::InvoiceRow;
use diesel_models::plans::PlanRow;
use diesel_models::query::billing_controller::CreditLapses;
use diesel_models::subscription_changes::SubscriptionChangeRow;
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscription_facts::{ENDING_CANCEL, SubscriptionEndingRow};
use diesel_models::subscriptions::{SubscriptionCycleRowPatch, SubscriptionRow};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

use crate::StoreResult;
use crate::domain::entity_activity::{Activity, ActivityType, AuditInput, EntityType};
use crate::domain::enums::BillingPeriodEnum;
use crate::domain::outbox_event::SubscriptionUpdateType;
use crate::domain::price_components::{PriceComponent, charges_nothing};
use crate::domain::prices::Pricing;
use crate::domain::{DetailedInvoice, SubscriptionDetails, SubscriptionStatusEnum};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::entity_activity::EntityActivityInterface;
use crate::services::credit::coupon_grant::has_started;
use crate::services::evaluate::bill::midnight;
use crate::services::evaluate::load::load_facts;
use crate::services::evaluate::pricer::{StoreFee, StorePricer};
use crate::services::invoice_lines::fees::pricing_error;
use crate::services::subscriptions::plan_change_credit::CancellationRecord;
use crate::services::subscriptions::start::{StartTransition, mark_subscription_started_at_tx};
use crate::services::{InvoiceBillingMode, InvoiceDelivery, Services, ServicesEdge};
use crate::store::PgConn;

/// Adjustment documents due with no change left to the controller: none by construction
/// (`written_slots`), so one is a failure of the subscription, never written.
static UNWRITTEN_ADJUSTMENTS: std::sync::LazyLock<opentelemetry::metrics::Counter<u64>> =
    std::sync::LazyLock::new(|| {
        common_logging::GLOBAL_METER
            .u64_counter("billing_controller_unwritten_adjustments")
            .with_description("Adjustment documents due by the facts that no change wrote")
            .build()
    });

/// Changes whose facts were written without their documents, settled by the controller.
static ADJUSTMENTS_SETTLED: std::sync::LazyLock<opentelemetry::metrics::Counter<u64>> =
    std::sync::LazyLock::new(|| {
        common_logging::GLOBAL_METER
            .u64_counter("billing_controller_settled_changes")
            .with_description("Changes left without their documents that the controller settled")
            .build()
    });

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::<Report<StoreError>>::into)
}

/// The derived status a stored one stands for.
pub(crate) fn stored_as(status: &SubscriptionStatusEnum) -> Option<Status> {
    match status {
        SubscriptionStatusEnum::PendingActivation => Some(Status::Pending),
        SubscriptionStatusEnum::TrialActive => Some(Status::Trialing),
        SubscriptionStatusEnum::TrialExpired => Some(Status::TrialExpired),
        SubscriptionStatusEnum::Active => Some(Status::Active),
        SubscriptionStatusEnum::Cancelled | SubscriptionStatusEnum::Aborted => {
            Some(Status::Cancelled)
        }
        SubscriptionStatusEnum::Completed => Some(Status::Completed),
    }
}

/// The column a derived status is stored as.
fn status_row(status: Status) -> diesel_models::enums::SubscriptionStatusEnum {
    use diesel_models::enums::SubscriptionStatusEnum as Row;
    match status {
        Status::Pending => Row::PendingActivation,
        Status::Trialing => Row::TrialActive,
        Status::TrialExpired => Row::TrialExpired,
        Status::Active => Row::Active,
        Status::Cancelled => Row::Cancelled,
        Status::Completed => Row::Completed,
    }
}

fn is_live(status: Status) -> bool {
    !matches!(status, Status::Cancelled | Status::Completed)
}

/// As `has_started` reads the stored row: started, and not awaiting its checkout (OQ9).
fn has_started_as(status: Status, pending_checkout: bool) -> bool {
    match status {
        Status::Active => true,
        Status::Trialing => !pending_checkout,
        _ => false,
    }
}

fn cadence_of(months: u32) -> Option<BillingPeriodEnum> {
    [
        BillingPeriodEnum::Monthly,
        BillingPeriodEnum::Quarterly,
        BillingPeriodEnum::Semiannual,
        BillingPeriodEnum::Annual,
    ]
    .into_iter()
    .find(|cadence| cadence.as_months() == months)
}

/// Whether the controller writes a document due: a free plan's billing nothing never (R-104),
/// a slot whose usage is still to read being no zero, and the opening invoice of a start still
/// to settle, the checkout's, only once a free trial expired and it can be collected unattended
/// or awaits its payment (R-179).
pub(crate) fn writes_document(
    due: &Due,
    reads_usage: bool,
    free_plan: bool,
    collects_unattended: bool,
    status: Status,
) -> bool {
    if free_plan && !reads_usage && bills_nothing(due) {
        return false;
    }
    !due.awaiting_checkout || (status == Status::TrialExpired && collects_unattended)
}

/// Every charge is zero and grants no credit: included credit is minted from the issued invoice.
fn bills_nothing(due: &Due) -> bool {
    due.charges
        .iter()
        .all(|charge| charge.amount == 0 && charge.credit_grant.is_none())
}

/// See [`Services::plan_is_free`].
pub(crate) async fn plan_is_free_tx(
    conn: &mut PgConn,
    plan_version: PlanVersionId,
    tenant_id: TenantId,
) -> StoreResult<bool> {
    let plan = db(PlanRow::get_with_version(conn, plan_version, tenant_id).await)?;
    if plan.plan.plan_type == PlanTypeEnum::Free {
        return Ok(true);
    }
    let components =
        crate::repositories::price_components::price_components_of(conn, plan_version, tenant_id)
            .await?;
    Ok(priced_at_nothing(&components))
}

/// The plan version in force on `day`: taken by the latest switch on or before it, else left by
/// the first one, else `current`.
fn version_on(
    switches: &[(NaiveDate, PlanVersionId, PlanVersionId)],
    day: NaiveDate,
    current: PlanVersionId,
) -> PlanVersionId {
    switches
        .iter()
        .rev()
        .find(|(on, _, _)| *on <= day)
        .map(|(_, _, taken)| *taken)
        .or_else(|| switches.first().map(|(_, left, _)| *left))
        .unwrap_or(current)
}

/// Bills nothing and grants no credit, in any option and currency: a period of it writes nothing.
pub(crate) fn priced_at_nothing(components: &[PriceComponent]) -> bool {
    charges_nothing(components) && !components.iter().any(includes_credit)
}

/// A credit pack, paid or included.
fn includes_credit(component: &PriceComponent) -> bool {
    let credit = |pricing: &Pricing| matches!(pricing, Pricing::Credit { .. });
    component.prices.iter().any(|price| credit(&price.pricing))
        || component.legacy_pricing.as_ref().is_some_and(|legacy| {
            legacy
                .pricing_entries
                .iter()
                .any(|(_, pricing)| credit(pricing))
        })
}

/// How a step issues the documents due.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Issue {
    /// None: a fact edit's own transaction writes the state, unless the subscription ends or a
    /// document is due past the stored period, which the controller does; woken at once for a
    /// document due.
    StateOnly,
    /// The controller's: finalized after their grace.
    AfterGrace,
    /// A replay at a past moment: settled inline, issued then.
    SettleAt(DateTime<Utc>),
}

/// What a step wrote.
pub(crate) struct Stepped {
    pub step: Step,
    /// The documents written, in their order.
    pub invoices: Vec<DetailedInvoice>,
}

impl Stepped {
    /// A transition applied, the state moved (a renewal writing no document) or a document
    /// written: what they made due runs next.
    pub(crate) fn took_work(&self) -> bool {
        !self.step.transitions.is_empty() || self.step.state.is_some() || !self.invoices.is_empty()
    }
}

type Settling = Option<(CancellationRecord, Vec<CreditNoteId>)>;

impl Services {
    /// Writes the derived state at the fact edit's instant `at` (the clock when later), in the
    /// caller's transaction under the customer's lock; the documents due are the controller's. A
    /// change dated later than the clock applies at its wake. Run again, it writes nothing.
    pub(crate) async fn derive_state_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        at: DateTime<Utc>,
    ) -> StoreResult<Option<Step>> {
        let stepped = self
            .step_subscription_tx(
                conn,
                actor,
                tenant_id,
                subscription_id,
                at.min(common_clock::now()),
                Issue::StateOnly,
            )
            .await?;
        Ok(stepped.map(|stepped| stepped.step))
    }

    /// One step of the subscription at `now`, under the customer's lock: the transitions since
    /// the stored state with their logs and outbox, the state, the documents due (per `issue`)
    /// and the next wake. None for a subscription no longer live.
    pub(crate) async fn step_subscription_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        now: DateTime<Utc>,
        issue: Issue,
    ) -> StoreResult<Option<Stepped>> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(self.step_unboxed(conn, actor, tenant_id, subscription_id, now, issue)).await
    }

    async fn step_unboxed(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        now: DateTime<Utc>,
        issue: Issue,
    ) -> StoreResult<Option<Stepped>> {
        // A fact edit's own derive of the state is not a step the tests fault: a fault lands in the
        // step after the edit commits.
        #[cfg(all(feature = "test-faults", debug_assertions))]
        if !matches!(issue, Issue::StateOnly) {
            crate::services::step_faults::step_of(subscription_id);
        }
        let row =
            db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
                .subscription;
        let Some(stored_status) = stored_as(&SubscriptionStatusEnum::from(row.status.clone()))
            .filter(|status| is_live(*status))
        else {
            if row.next_wake_at.is_some() {
                db(SubscriptionRow::set_next_wake_at(conn, subscription_id, None).await)?;
            }
            return Ok(None);
        };
        // One boundary a step: a late controller writes the stored period's renewal, then runs
        // again at once for the next, each period's documents and transitions at their instant.
        // A subscription still to start goes no further than its start.
        let boundary = match row.current_period_end {
            Some(end) => Some(end.max(row.start_date)),
            None if stored_status == Status::Pending => Some(row.start_date),
            None => None,
        };
        // Past its start with an open-ended period, it is evaluated at its start until it settles.
        let held_at_start = row.current_period_end.is_none()
            && stored_status == Status::Pending
            && midnight(row.start_date) <= now;
        let now = boundary
            .map(midnight)
            .map_or(now, |boundary| now.min(boundary));
        // The trial's end decided once reached, before the facts it writes are read (R-167).
        let row =
            if crate::services::trials::ending::decide_trial_end_tx(self, conn, actor, &row, now)
                .await?
                .is_some()
            {
                db(
                    SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                        .await,
                )?
                .subscription
            } else {
                row
            };

        // The grid each change of cadence runs on, read from the period as stored before it
        // moves, and before the facts are read: the plan windows hold the origins it records.
        // A cancellation due inside the stored period reads that period's grid, as it began.
        let origins_on = match row.current_period_end {
            Some(end) if end <= now.date_naive() => Some(end),
            Some(_) => db(SubscriptionEndingRow::list_of(conn, subscription_id).await)?
                .iter()
                .any(|ending| ending.kind == ENDING_CANCEL && ending.at <= now.naive_utc())
                .then_some(row.current_period_start),
            None => None,
        };
        if let Some(on) = origins_on {
            let lock = db(SubscriptionRow::lock_customer_of(conn, subscription_id).await)?;
            crate::services::invoice_lines::cadence::record_cadence_origins_tx(
                conn,
                &lock,
                crate::services::invoice_lines::cadence::CadenceSubject::of_row(&row),
                on,
            )
            .await?;
        }

        defer_amendments_tx(conn, &row, now).await?;
        self.realign_amendments_tx(conn, &row, now).await?;
        let mut loaded =
            load_facts(conn, tenant_id, subscription_id, now.date_naive(), None).await?;
        // A start settled by the time the step runs is settled at the instant it evaluates, as an
        // invoice reads it: a checkout paid today activates a backdated subscription.
        if let Some(settled) = loaded.facts.start.settled
            && settled <= common_clock::now()
        {
            loaded.facts.start.settled = Some(settled.min(now));
        }
        let facts = &loaded.facts;
        let evaluation = evaluate(facts, now, None, &StorePricer).map_err(pricing_error)?;
        let stored_period = BillingPeriodEnum::from(row.period.clone());
        let stored = Stored {
            status: stored_status,
            current_period: Some((row.current_period_start, row.current_period_end)),
            plan_version: Some(row.plan_version_id),
            cadence: Some(stored_period.as_months()),
            pending_checkout: row.pending_checkout,
        };
        let written = written_slots(conn, tenant_id, subscription_id, &evaluation).await?;
        let invoice_deadlines =
            db(InvoiceRow::deadlines_of_subscription(conn, tenant_id, subscription_id).await)?;
        let finalize_after: Vec<DateTime<Utc>> = invoice_deadlines
            .iter()
            .filter_map(|(at, _)| at.map(|at| at.and_utc()))
            .collect();
        let attempts: Vec<DateTime<Utc>> = invoice_deadlines
            .iter()
            .filter_map(|(_, at)| at.map(|at| at.and_utc()))
            .collect();
        let credit = db(CreditLapses::of_customer(conn, tenant_id, row.customer_id).await)?;
        let deadlines = Deadlines {
            finalize_after: &finalize_after,
            next_payment_attempt_at: &attempts,
            credit_lapses: &credit.lapses,
            settlement_grace: crate::repositories::credit::customer_credit::settlement_grace(
                credit.grace_period_hours.unwrap_or(0),
                credit.consolidate_recurring_invoices.unwrap_or(false),
            ),
        };
        let step = step(facts, now, &evaluation, &stored, &written, &deadlines);
        let derived = Stored::from(&evaluation.state);
        // Only its checkout or its activation moves a start awaiting them, and each marks the
        // customer: no wake until then, which `DueTimes` counts as none.
        let awaits_its_start =
            held_at_start && derived.status == Status::Pending && !facts.settled_by(now);
        let ending = facts.effective_ending();

        let reading: Vec<DocumentSlot> = evaluation
            .usage_needed
            .iter()
            .map(|need| need.document)
            .collect();
        // A start settled without a card awaits its opening invoice's payment (R-179); one still
        // awaiting its checkout pays there.
        let awaits_payment = !facts.start.checkout
            && facts
                .free_trial()
                .is_some_and(|trial| trial.after == billing_core::facts::AfterTrial::AwaitsPayment);
        let documents = self
            .documents_to_write(conn, &row, &step, &reading, &derived, awaits_payment)
            .await?;
        let ends = step.transitions.iter().any(|transition| {
            matches!(
                transition,
                Transition::Status {
                    to: Status::Cancelled | Status::Completed,
                    ..
                }
            )
        });
        // A period's document is the controller's alone, and an ending's documents go with it; an
        // adjustment is the change's own, written in its transaction.
        let periods_due = documents.iter().any(|due| match due.slot {
            DocumentSlot::Recurring { .. } | DocumentSlot::Final => true,
            DocumentSlot::Adjustment { .. } => false,
        });
        // Without its documents the state moves only up to the stored period's end: a renewal
        // past it is the controller's, written with that state. A change may recut the period.
        let crosses = match (derived.current_period, stored.current_period) {
            (Some((start, _)), Some((_, Some(end)))) => start >= end,
            (derived, stored) => derived != stored,
        };
        // A switch written ahead settles the old plan's ending, reading metering: the controller's
        // step reads it before its locks, a fact edit's transaction does not.
        if matches!(issue, Issue::StateOnly)
            && (ends
                || (periods_due && crosses)
                || settles_a_switch(conn, &row, &step.transitions).await?)
        {
            db(SubscriptionRow::set_next_wake_at(conn, subscription_id, Some(now)).await)?;
            return Ok(Some(Stepped {
                step,
                invoices: Vec::new(),
            }));
        }

        // What an ending settles reads the period it ends, before the state moves past it.
        let mut cancelled: Option<(SubscriptionDetails, DateTime<Utc>)> = None;
        for transition in &step.transitions {
            match *transition {
                Transition::Status {
                    to: Status::Cancelled,
                    at,
                    ..
                } => {
                    cancelled = Some(self.end_cancelled_tx(conn, &row, facts, ending, at).await?);
                }
                Transition::Status {
                    to: Status::Completed,
                    at,
                    ..
                } => self.end_ended_tx(conn, &row, at).await?,
                _ => {}
            }
        }
        self.write_state_tx(conn, &row, &stored, &derived, &step, ending)
            .await?;

        let switches: Vec<(PlanVersionId, PlanVersionId, NaiveDate)> = step
            .transitions
            .iter()
            .filter_map(|transition| match transition {
                Transition::Plan { from, to, at } => Some((*from, *to, at.date_naive())),
                Transition::Status { .. } => None,
            })
            .collect();
        for (from, to, on) in &switches {
            self.record_switch_tx(conn, actor, &row, *from, *to, *on, &stored_period)
                .await?;
        }
        let amended = self.record_amendments_tx(conn, &row, now).await?;
        let plan_version = derived.plan_version.unwrap_or(row.plan_version_id);
        let cadence = derived
            .cadence
            .and_then(cadence_of)
            .unwrap_or(stored_period);
        if plan_version != row.plan_version_id {
            crate::services::subscriptions::utils::rescope_coupons_to_plan_tx(
                conn,
                subscription_id,
                plan_version,
            )
            .await?;
        }
        if plan_version != row.plan_version_id || cadence != stored_period {
            db(SubscriptionRow::update_plan_version(
                conn,
                &subscription_id,
                &tenant_id,
                plan_version,
                Some(cadence.into()),
            )
            .await)?;
        }
        // A switch to a free plan leaves the debts carried to its next invoice stranded (R-104).
        for (_, to, on) in &switches {
            if self.plan_is_free(conn, *to, tenant_id).await? {
                let on = on.and_time(chrono::NaiveTime::MIN).and_utc();
                self.reopen_stranded_debts_tx(conn, &row, on, now).await?;
            }
        }
        // A change to another option of the same version switches no plan, and may keep the
        // cadence: recorded as the period it starts begins.
        if plan_version == row.plan_version_id
            && derived.current_period != stored.current_period
            && let Some((start, _)) = derived.current_period
            && let Some(change) = written_ahead_change(conn, &row, plan_version, start).await?
        {
            record_changed_option(&self.store, conn, &row, &change, None).await?;
        }

        // The fees in force today, restated before the documents that read it: a change at the
        // period's end moves the MRR with the renewal that applies it.
        let today = now.date_naive();
        let mrr_on = switches
            .last()
            .map(|(_, _, on)| *on)
            .or_else(|| latest_change(facts, today))
            .unwrap_or(today);
        if is_live(derived.status) {
            self.restate_mrr_tx(conn, tenant_id, subscription_id, today, mrr_on)
                .await?;
        }

        let first_period = evaluation.state.first_period.map(|(start, _)| start);
        let mut invoices = Vec::new();
        let issued = if matches!(issue, Issue::StateOnly) {
            &[][..]
        } else {
            &documents[..]
        };
        let opened = issued.iter().any(|due| {
            matches!(due.slot, DocumentSlot::Recurring { period_start }
                if Some(period_start) == first_period)
        });
        // Final invoices off, the ending's day invoice written in this step already carries it.
        let ending_day_written = ending.is_some_and(|ending| {
            issued.iter().any(|due| {
                due.slot
                    == DocumentSlot::Recurring {
                        period_start: ending.on(),
                    }
            })
        });
        // A backdated cancellation's drafts of periods after its ending close before the final
        // invoice reads what is billed: their charges are its own (R-016).
        if cancelled.is_some()
            && let Some(ending) = ending
            && facts
                .ending_recorded
                .is_some_and(|recorded| recorded > ending.on())
        {
            for draft in db(InvoiceRow::recurring_drafts_from(
                conn,
                tenant_id,
                subscription_id,
                ending.on(),
            )
            .await)?
            {
                self.close_draft_past_ending_tx(conn, actor, tenant_id, draft)
                    .await?;
            }
        }
        for due in issued {
            if due.slot == DocumentSlot::Final
                && ending_day_written
                && !crate::services::subscriptions::terminate::issues_final_invoices()
            {
                continue;
            }
            if let Some(invoice) = self
                .write_document_tx(conn, actor, &row, due, issue, ending)
                .await?
            {
                invoices.push(invoice);
            }
        }
        // A cancellation billing nothing else writes its document when it trues up again an
        // earlier period it gives spend back in (R-022).
        if cancelled.is_some()
            && let Some(ending) = ending.filter(|ending| ending.kind == EndingKind::Cancel)
        {
            let backdated = facts
                .ending_recorded
                .is_some_and(|recorded| recorded > ending.on());
            let slot = if backdated || facts.ending_day_invoiced {
                DocumentSlot::Final
            } else {
                DocumentSlot::Recurring {
                    period_start: ending.on(),
                }
            };
            if !written.contains(&slot) && !issued.iter().any(|due| due.slot == slot) {
                let details = self
                    .store
                    .get_subscription_details_with_conn(conn, row.tenant_id, row.id)
                    .await?;
                if self
                    .ending_reopens_true_ups(conn, &details, ending.on())
                    .await?
                {
                    let due = Due {
                        slot,
                        at: ending.at,
                        charges: Vec::new(),
                        awaiting_checkout: false,
                    };
                    if let Some(invoice) = self
                        .write_document_tx(conn, actor, &row, &due, issue, Some(ending))
                        .await?
                    {
                        invoices.push(invoice);
                    }
                }
            }
        }
        // The cancellation's give-back follows the documents due: one written now bills its
        // period as of its date, whole, against the lines it gives back (R-077).
        let mut settling: Settling = match cancelled {
            Some((details, ended)) => {
                self.issue_cancellation_refund(conn, actor, &details, ended.date_naive(), ended)
                    .await?
            }
            None => None,
        };
        let starts = step.transitions.iter().any(|transition| {
            matches!(
                transition,
                Transition::Status {
                    from: Status::Pending,
                    to: Status::Active | Status::Trialing,
                    ..
                }
            )
        });
        if starts
            && !matches!(issue, Issue::StateOnly)
            && let Some(start) = first_period
            && let Some(invoice) = self
                .issue_opening_draft_tx(conn, actor, &row, start, issue)
                .await?
        {
            invoices.push(invoice);
        }

        for transition in &step.transitions {
            match *transition {
                Transition::Status {
                    from: Status::Trialing,
                    to: Status::Active | Status::TrialExpired,
                    ..
                } => {
                    self.emit_subscription_updated_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                        SubscriptionUpdateType::TrialEnded,
                    )
                    .await?;
                }
                Transition::Status {
                    to: Status::Cancelled,
                    at,
                    ..
                } => {
                    self.cancelled_tx(conn, actor, &row, at, settling.take())
                        .await?;
                }
                Transition::Status {
                    to: Status::Completed,
                    at,
                    ..
                } => self.completed_tx(conn, &row, at).await?,
                _ => {}
            }
        }
        if let Some(ended) = step
            .transitions
            .iter()
            .find_map(|transition| match transition {
                Transition::Status {
                    to: Status::Cancelled | Status::Completed,
                    at,
                    ..
                } => Some(*at),
                _ => None,
            })
        {
            self.reopen_stranded_debts_tx(conn, &row, ended, now)
                .await?;
        }
        let activated = step.transitions.iter().any(|transition| {
            matches!(
                transition,
                Transition::Status {
                    to: Status::Active,
                    ..
                }
            )
        });
        if opened || activated {
            self.book_created_mrr_tx(conn, tenant_id, subscription_id)
                .await?;
            // The creation's New Business, booked only now, is what the restate above waited for.
            if is_live(derived.status) {
                self.restate_mrr_tx(conn, tenant_id, subscription_id, today, mrr_on)
                    .await?;
            }
        }

        let wake = if matches!(issue, Issue::StateOnly) && periods_due {
            Some(now)
        } else if awaits_its_start {
            None
        } else {
            step.next_wake_at
                .filter(|wake| *wake > now && is_live(derived.status))
        };
        if wake != row.next_wake_at {
            db(SubscriptionRow::set_next_wake_at(conn, subscription_id, wake).await)?;
        }
        if plan_version != row.plan_version_id || !switches.is_empty() {
            self.emit_subscription_updated_tx(
                conn,
                tenant_id,
                subscription_id,
                SubscriptionUpdateType::PlanChanged,
            )
            .await?;
        } else if amended {
            self.emit_subscription_updated_tx(
                conn,
                tenant_id,
                subscription_id,
                SubscriptionUpdateType::Amended,
            )
            .await?;
        }
        Ok(Some(Stepped { step, invoices }))
    }

    /// The documents due the controller writes, of those `step` lists.
    async fn documents_to_write(
        &self,
        conn: &mut PgConn,
        row: &SubscriptionRow,
        step: &Step,
        reading: &[DocumentSlot],
        derived: &Stored,
        awaits_payment: bool,
    ) -> StoreResult<Vec<Due>> {
        let due = &step.documents;
        if due.is_empty() {
            return Ok(Vec::new());
        }
        let plan_version = derived.plan_version.unwrap_or(row.plan_version_id);
        let zero_due =
            |document: &Due| !reading.contains(&document.slot) && bills_nothing(document);
        // R-104 per document, on the plan version in force on its day: a free plan's periods
        // stay unwritten once it is changed to a paid one.
        let mut free_plan = Vec::with_capacity(due.len());
        if due.iter().any(zero_due) {
            // This step's switches are recorded after its documents are written.
            let mut switches = db(SubscriptionEventRow::plan_switches(conn, row.id).await)?;
            for transition in &step.transitions {
                if let Transition::Plan { from, to, at } = transition
                    && from != to
                    && !switches.contains(&(at.date_naive(), *from, *to))
                {
                    switches.push((at.date_naive(), *from, *to));
                }
            }
            switches.sort_by_key(|(on, _, _)| *on);
            let mut free_of: HashMap<PlanVersionId, bool> = HashMap::new();
            for document in due {
                let version = version_on(&switches, document.at.date_naive(), plan_version);
                let free = match free_of.get(&version) {
                    Some(free) => *free,
                    None => {
                        let free = self.plan_is_free(conn, version, row.tenant_id).await?;
                        free_of.insert(version, free);
                        free
                    }
                };
                free_plan.push(free);
            }
        } else {
            free_plan.resize(due.len(), false);
        }
        let collects_unattended = row.charge_automatically
            && due.iter().any(|document| document.awaiting_checkout)
            && !db(CustomerPaymentMethodRow::list_by_customer_id(
                conn,
                &row.tenant_id,
                &row.customer_id,
            )
            .await)?
            .is_empty();
        Ok(due
            .iter()
            .zip(free_plan)
            .filter(|(document, free_plan)| {
                writes_document(
                    document,
                    reading.contains(&document.slot),
                    *free_plan,
                    collects_unattended || awaits_payment,
                    derived.status,
                )
            })
            .map(|(document, _)| document.clone())
            .collect())
    }

    /// Free as R-104 reads it: typed free, or priced at nothing with no credit pack (granted
    /// from its zero invoice). A typed free plan's credit lines are kept by the caller.
    pub(in crate::services) async fn plan_is_free(
        &self,
        conn: &mut PgConn,
        plan_version: PlanVersionId,
        tenant_id: TenantId,
    ) -> StoreResult<bool> {
        plan_is_free_tx(conn, plan_version, tenant_id).await
    }

    /// Writes one document due; None when it bills nothing.
    async fn write_document_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        row: &SubscriptionRow,
        due: &Due,
        issue: Issue,
        ending: Option<Ending>,
    ) -> StoreResult<Option<DetailedInvoice>> {
        let mode = match issue {
            Issue::SettleAt(at) => InvoiceBillingMode::SettleAt(at),
            Issue::StateOnly | Issue::AfterGrace => {
                InvoiceBillingMode::AwaitGracePeriodIfApplicable
            }
        };
        match due.slot {
            DocumentSlot::Recurring { period_start } => {
                self.bill_period_tx(conn, actor, row.tenant_id, row.id, period_start, mode)
                    .await
            }
            DocumentSlot::Final
                if crate::services::subscriptions::terminate::issues_final_invoices() =>
            {
                let written = self
                    .bill_final_invoice_tx(conn, actor, row.tenant_id, row.id)
                    .await?;
                // Its day's invoice issued before the ending, the ending's hours are this one's.
                if written.is_none()
                    && let Some(ending) = ending
                    && self.ending_day_issued(conn, row, ending.on()).await?
                {
                    return Err(Report::new(StoreError::BillingInvariantViolated(format!(
                        "subscription {}: a final invoice is due after the invoice of {} was issued, and none was written",
                        row.id,
                        ending.on()
                    ))));
                }
                Ok(written)
            }
            // Final invoices off, its charges ride the recurring invoice of the ending's day.
            DocumentSlot::Final => Ok(None),
            DocumentSlot::Adjustment { on } => {
                let tenant = [common_logging::KeyValue::new(
                    "tenant",
                    row.tenant_id.to_string(),
                )];
                match db(SubscriptionChangeRow::take_settlement_due(conn, row.id, on).await)? {
                    Some(kind) => {
                        ADJUSTMENTS_SETTLED.add(1, &tenant);
                        self.settle_adjustment_tx(conn, actor, row, on, kind).await
                    }
                    None => {
                        UNWRITTEN_ADJUSTMENTS.add(1, &tenant);
                        let charges: Vec<String> = due
                            .charges
                            .iter()
                            .map(|c| format!("{:?} {:?} {}", c.key.fee, c.key.kind, c.amount))
                            .collect();
                        Err(Report::new(StoreError::BillingInvariantViolated(format!(
                            "subscription {}: an adjustment dated {on} is due by the facts and no change wrote it: {}",
                            row.id,
                            charges.join(", ")
                        ))))
                    }
                }
            }
        }
    }

    /// The change dated `on` whose facts were written without its documents, settled as that
    /// change settles itself: a plan change gives back by unused time, an amendment latest
    /// first (R-077, R-078).
    async fn settle_adjustment_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        row: &SubscriptionRow,
        on: NaiveDate,
        kind: SubscriptionChangeKindEnum,
    ) -> StoreResult<Option<DetailedInvoice>> {
        use crate::domain::enums::CreditNoteOrigin;
        use crate::repositories::InvoiceInterface;
        use crate::services::evaluate::change::{Change, GiveBack};
        let (change, reason, origin) = if kind == SubscriptionChangeKindEnum::PlanChange {
            (
                Change::of_rows(on, GiveBack::Unused, "plan change"),
                "Plan change",
                CreditNoteOrigin::PlanChange,
            )
        } else {
            (
                Change::of_rows(on, GiveBack::LatestFirst, "amendment").without_packs(),
                "Subscription amendment",
                CreditNoteOrigin::Amendment,
            )
        };
        let details = self
            .store
            .get_subscription_details_with_conn(conn, row.tenant_id, row.id)
            .await?;
        let (invoice, _notes) = self
            .settle_change_tx(
                conn,
                actor,
                &details.subscription,
                &details.customer,
                &change,
                reason,
                origin,
            )
            .await?;
        match invoice {
            Some(invoice_id) => self
                .store
                .get_detailed_invoice_by_id_with_conn(conn, row.tenant_id, invoice_id)
                .await
                .map(Some),
            None => Ok(None),
        }
    }

    /// The opening invoice an earlier release wrote ahead of the start and left a draft, with no
    /// finalization scheduled: the start issues it as it would one written now.
    async fn issue_opening_draft_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        row: &SubscriptionRow,
        start: NaiveDate,
        issue: Issue,
    ) -> StoreResult<Option<DetailedInvoice>> {
        let left =
            db(
                InvoiceRow::find_existing_recurring_invoice(conn, row.tenant_id, row.id, start)
                    .await,
            )?
            .filter(|invoice| {
                invoice.status == InvoiceStatusEnum::Draft
                    && invoice.auto_advance
                    && invoice.finalize_after.is_none()
                    && invoice.consolidated_into_invoice_id.is_none()
            });
        if left.is_none() {
            return Ok(None);
        }
        let due = Due {
            slot: DocumentSlot::Recurring {
                period_start: start,
            },
            at: midnight(start),
            charges: Vec::new(),
            awaiting_checkout: false,
        };
        self.write_document_tx(conn, actor, row, &due, issue, None)
            .await
    }

    /// Whether the recurring invoice of `on` was issued: past it, an ending's hours that day are a
    /// final invoice's.
    async fn ending_day_issued(
        &self,
        conn: &mut PgConn,
        row: &SubscriptionRow,
        on: NaiveDate,
    ) -> StoreResult<bool> {
        Ok(
            db(InvoiceRow::find_existing_recurring_invoice(conn, row.tenant_id, row.id, on).await)?
                .is_some_and(|invoice| invoice.status != InvoiceStatusEnum::Draft),
        )
    }

    /// The state columns the step derives, a subscription starting through the one start path.
    async fn write_state_tx(
        &self,
        conn: &mut PgConn,
        row: &SubscriptionRow,
        stored: &Stored,
        derived: &Stored,
        step: &Step,
        ending: Option<Ending>,
    ) -> StoreResult<()> {
        let cancelled_now =
            derived.status == Status::Cancelled && stored.status != Status::Cancelled;
        // An immediate cancellation's instant; one at midnight settles at the date's boundary.
        let terminated_at = ending
            .filter(|ending| {
                cancelled_now
                    && ending.kind == EndingKind::Cancel
                    && ending.at != midnight(ending.on())
            })
            .map(|ending| Some(ending.at.naive_utc()));
        if step.state.is_none() && terminated_at.is_none() {
            return Ok(());
        }
        let patch = SubscriptionCycleRowPatch {
            id: row.id,
            tenant_id: row.tenant_id,
            status: (derived.status != stored.status).then(|| status_row(derived.status)),
            current_period_start: derived.current_period.map(|(start, _)| start),
            current_period_end: derived.current_period.map(|(_, end)| end),
            pending_checkout: Some(derived.pending_checkout),
            billing_start_date: None,
            billing_day_anchor: None,
            terminated_at,
        };
        if !has_started(row) && has_started_as(derived.status, derived.pending_checkout) {
            let at = step
                .transitions
                .iter()
                .find_map(|transition| match transition {
                    Transition::Status {
                        to: Status::Active | Status::Trialing,
                        at,
                        ..
                    } => Some(*at),
                    _ => None,
                })
                .unwrap_or_else(common_clock::now);
            return mark_subscription_started_at_tx(
                conn,
                &*self.store.hooks.credit,
                row.tenant_id,
                row.id,
                StartTransition::Cycle(patch),
                at,
            )
            .await;
        }
        db(patch.patch(conn).await)
    }

    /// The period a cancellation ends, as its give-back reads it (R-077, R18), and what every
    /// ending does; the give-back itself follows the step's documents.
    async fn end_cancelled_tx(
        &self,
        conn: &mut PgConn,
        row: &SubscriptionRow,
        facts: &Facts<StoreFee>,
        ending: Option<Ending>,
        at: DateTime<Utc>,
    ) -> StoreResult<(SubscriptionDetails, DateTime<Utc>)> {
        let ended = ending.map_or(at, |ending| ending.at);
        let mut details = self
            .store
            .get_subscription_details_with_conn(conn, row.tenant_id, row.id)
            .await?;
        // The period holding the ending, however late the step runs: on a period's first
        // midnight, that period, billed already and never used. Backdated before the stored
        // period, it reaches to that period's end: the periods billed since come back too.
        let unended = Facts {
            ending: None,
            ..facts.clone()
        };
        if let Some((start, end)) = evaluate(&unended, ended, None, &StorePricer)
            .map_err(pricing_error)?
            .state
            .current_period
        {
            details.subscription.current_period_start = start;
            details.subscription.current_period_end = match (end, row.current_period_end) {
                (Some(end), Some(stored)) => Some(end.max(stored)),
                (end, _) => end,
            };
        }
        self.end_ended_tx(conn, row, ended).await?;
        Ok((details, ended))
    }

    /// What every ending does at its instant: changes still pending withdrawn, the
    /// subscription's credit ended before its last documents settle (OQ18).
    async fn end_ended_tx(
        &self,
        conn: &mut PgConn,
        row: &SubscriptionRow,
        ended: DateTime<Utc>,
    ) -> StoreResult<()> {
        crate::services::facts::pending::withdraw_pending_of_tx(
            conn,
            row.tenant_id,
            row.id,
            crate::services::facts::pending::CHANGES,
            "Subscription terminated",
        )
        .await?;
        crate::services::subscriptions::phases::withdraw_future_phases_tx(
            conn,
            row.tenant_id,
            row.id,
            ended.date_naive(),
            "Subscription terminated",
        )
        .await?;
        crate::services::subscriptions::timeline_items::withdraw_future_items_tx(
            conn,
            row.tenant_id,
            row.id,
            ended.date_naive(),
            "Subscription terminated",
        )
        .await?;
        crate::services::credit::subscription_end::end_subscription_credit_tx(
            conn,
            &*self.store.hooks.credit,
            row.tenant_id,
            row.id,
            ended,
            common_clock::now(),
        )
        .await?;
        crate::services::offers::end_with_subscription_tx(
            conn,
            row.tenant_id,
            row.id,
            ended.naive_utc(),
        )
        .await?;
        Ok(())
    }

    /// A cancellation's logs and outbox, once its documents are written: the churn its
    /// `Cancelled` event carries, unless an invoice booked it.
    async fn cancelled_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        row: &SubscriptionRow,
        at: DateTime<Utc>,
        settling: Settling,
    ) -> StoreResult<()> {
        let date = at.date_naive();
        if let Err(error) = self
            .create_churn_mrr_log(conn, row.tenant_id, row.id, date)
            .await
        {
            log::error!(
                "Failed to create churn MRR log for subscription {}: {error:?}",
                row.id
            );
        }
        if let Some((record, notes)) = settling {
            self.record_settling_cancellation_tx(conn, row.id, date, &record, &notes)
                .await?;
        }
        let activity = Activity::new(
            ActivityType::SubscriptionCancelled,
            EntityType::Subscription,
            row.id.as_uuid(),
        )
        .agg_customer(row.customer_id);
        self.store
            .record_tx(conn, row.tenant_id, actor, AuditInput::Activity(activity))
            .await?;
        let reason = db(SubscriptionEndingRow::list_of(conn, row.id).await)?
            .into_iter()
            .find(|ending| ending.kind == ENDING_CANCEL)
            .and_then(|ending| ending.reason);
        self.emit_subscription_cancelled_tx(conn, row.tenant_id, row.id, reason)
            .await
    }

    /// The debts carried to an invoice that will never come reopen on the invoices that carried
    /// them, after the ending's own documents took what they could.
    async fn reopen_stranded_debts_tx(
        &self,
        conn: &mut PgConn,
        row: &SubscriptionRow,
        ended: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> StoreResult<()> {
        let lock = db(SubscriptionRow::lock_customer_of(conn, row.id).await)?;
        let reopened = crate::services::invoices::carry_forward::reopen_stranded_debts_tx(
            conn,
            &lock,
            &row.currency,
            ended.date_naive(),
        )
        .await?;
        for invoice_id in reopened {
            self.store
                .internal
                .on_invoice_reopened(
                    conn,
                    row.tenant_id,
                    invoice_id,
                    false,
                    true,
                    crate::services::payment_reminders::sweep::ReopenedReminders::Restart,
                    now.naive_utc(),
                )
                .await?;
        }
        Ok(())
    }

    /// A fixed term's completion: its `Ended` event and churn, and the outbox.
    async fn completed_tx(
        &self,
        conn: &mut PgConn,
        row: &SubscriptionRow,
        at: DateTime<Utc>,
    ) -> StoreResult<()> {
        let current =
            db(SubscriptionRow::get_subscription_by_id(conn, &row.tenant_id, row.id).await)?
                .subscription;
        self.churn_ended_subscription(conn, &current, at.date_naive())
            .await?;
        self.emit_subscription_ended_tx(conn, row.tenant_id, row.id)
            .await
    }

    /// Each amendment written ahead taking effect by `at`, not yet recorded, that opened no plan
    /// window: written with the fees then in force, it moves the subscription onto the shortest
    /// cadence of those in force on its day, as an immediate amendment that day would (R-084).
    async fn realign_amendments_tx(
        &self,
        conn: &mut PgConn,
        row: &SubscriptionRow,
        at: DateTime<Utc>,
    ) -> StoreResult<()> {
        let in_force = in_force_written_ahead(conn, row, at).await?;
        for change in in_force {
            if db(SubscriptionEventRow::change_applied(conn, row.id, change.id).await)? {
                continue;
            }
            let opened_window = change
                .written_facts
                .clone()
                .and_then(|record| {
                    serde_json::from_value::<crate::services::facts::pending::WrittenFacts>(record)
                        .ok()
                })
                .is_some_and(|written| written.plan_window.is_some());
            if !opened_window {
                crate::services::facts::pending::record_amendment_cadence_tx(
                    conn,
                    row.tenant_id,
                    row.id,
                    change.effective_at.date(),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// The Switch row of each amendment written ahead that took effect by `at`, once; true when
    /// one took effect since the last step.
    async fn record_amendments_tx(
        &self,
        conn: &mut PgConn,
        row: &SubscriptionRow,
        at: DateTime<Utc>,
    ) -> StoreResult<bool> {
        let in_force = in_force_written_ahead(conn, row, at).await?;
        let mut amended = false;
        let mut renewed = Vec::new();
        for change in in_force {
            if db(SubscriptionEventRow::change_applied(conn, row.id, change.id).await)? {
                continue;
            }
            db(SubscriptionEventRow {
                id: uuid::Uuid::now_v7(),
                subscription_id: row.id,
                event_type: SubscriptionEventType::Switch,
                details: Some(serde_json::json!({
                    "kind": if change.kind == SubscriptionChangeKindEnum::Phase {
                        "phase"
                    } else {
                        "amendment"
                    },
                    "mode": "end_of_period",
                    "event": change.id.as_uuid().to_string(),
                })),
                created_at: common_clock::now().naive_utc(),
                mrr_delta: None,
                bi_mrr_movement_log_id: None,
                applies_to: change.effective_at.date(),
            }
            .insert(conn)
            .await)?;
            if change.kind == SubscriptionChangeKindEnum::Phase {
                renewed.push(change.clone());
            }
            // A longer period of a fee it changed gives back what was billed past its day, before
            // the renewal of that day bills.
            self.settle_scheduled_amendment_tx(
                conn,
                &Actor::System,
                row.tenant_id,
                row.id,
                change.effective_at.date(),
            )
            .await?;
            let activity = Activity::new(
                ActivityType::SubscriptionAmended,
                EntityType::Subscription,
                row.id.as_uuid(),
            )
            .with_metadata(serde_json::json!({
                "effective_date": change.effective_at.date().to_string(),
            }))
            .agg_customer(row.customer_id);
            self.store
                .record_tx(
                    conn,
                    row.tenant_id,
                    &Actor::System,
                    AuditInput::Activity(activity),
                )
                .await?;
            amended = true;
        }
        // A term renewing on one of these days writes its next renewal once every change of the
        // step is recorded: none still pending stands in its way (R-152).
        for change in &renewed {
            crate::services::subscriptions::terms::renewed_tx(self, conn, change).await?;
        }
        Ok(amended)
    }

    /// A plan transition's Switch row, signal and activity, once per (subscription, plan, day).
    #[allow(clippy::too_many_arguments)]
    async fn record_switch_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        row: &SubscriptionRow,
        from: PlanVersionId,
        to: PlanVersionId,
        on: NaiveDate,
        outgoing_period: &BillingPeriodEnum,
    ) -> StoreResult<()> {
        if db(SubscriptionEventRow::switch_recorded(conn, row.id, to, on).await)? {
            return Ok(());
        }
        // The period ending here is trued up under the terms it ran on.
        let outgoing_minimum = crate::repositories::subscriptions::load_subscription_minimum_on(
            conn,
            row.id,
            on.pred_opt().unwrap_or(on),
        )
        .await?;
        let mut details = serde_json::json!({
            "new_plan_version_id": to.to_string(),
            "old_plan_version_id": from.to_string(),
        });
        // The change written ahead it applies, no longer pending once named here.
        let scheduled = written_ahead_change(conn, row, to, on).await?;
        let written_ahead = scheduled.is_some();
        if let Some(change) = scheduled {
            record_changed_option(&self.store, conn, row, &change, Some((to, on))).await?;
            details["event"] = serde_json::Value::String(change.id.as_uuid().to_string());
            // Settled now and again once a draft billing an old fee is issued, as an immediate one.
            details["outgoing"] = serde_json::Value::String("settled".to_owned());
        }
        for (key, value) in crate::services::invoice_lines::minimum::outgoing_terms_record(
            outgoing_minimum.as_ref(),
            *outgoing_period,
        ) {
            details[key] = value;
        }
        db(SubscriptionEventRow {
            id: uuid::Uuid::now_v7(),
            subscription_id: row.id,
            event_type: SubscriptionEventType::Switch,
            details: Some(details),
            created_at: common_clock::now().naive_utc(),
            mrr_delta: None,
            bi_mrr_movement_log_id: None,
            applies_to: on,
        }
        .insert(conn)
        .await)?;
        // A change written ahead gives back what a fee of the old plan billed past the switch (a
        // year on a monthly period), as an immediate change settles itself that day (R-077).
        if written_ahead {
            self.settle_plan_change_ending_tx(conn, actor, row.tenant_id, row.id, on)
                .await?;
        }
        self.store
            .hooks
            .signals
            .subscription_plan_changed(
                conn,
                row.tenant_id,
                row.customer_id,
                row.id,
                common_clock::now().naive_utc(),
            )
            .await?;
        let activity = Activity::new(
            ActivityType::SubscriptionPlanChanged,
            EntityType::Subscription,
            row.id.as_uuid(),
        )
        .with_metadata(serde_json::json!({
            "new_plan_version_id": to.to_string(),
            "change_date": on.to_string(),
        }))
        .agg_customer(row.customer_id);
        self.store
            .record_tx(conn, row.tenant_id, actor, AuditInput::Activity(activity))
            .await
    }

    /// The MRR of the fees in force on `day`, its movement booked on `on`. Before the first
    /// period's New Business is booked the stored MRR is not the subscription's yet: nothing moves.
    async fn restate_mrr_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        day: NaiveDate,
        on: NaiveDate,
    ) -> StoreResult<()> {
        let row =
            db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
                .subscription;
        if !has_started(&row) {
            return Ok(());
        }
        let created = db(
            SubscriptionEventRow::fetch_by_subscription_id_and_event_type(
                conn,
                subscription_id,
                SubscriptionEventType::Created,
                row.start_date,
            )
            .await,
        )?;
        if created.is_some_and(|event| {
            event.mrr_delta.is_some_and(|delta| delta != 0)
                && event.bi_mrr_movement_log_id.is_none()
        }) {
            return Ok(());
        }
        let mut details = self
            .store
            .subscription_details_as_of(conn, tenant_id, subscription_id, day)
            .await?;
        let precision = crate::constants::Currencies::resolve_currency_precision(
            &details.subscription.currency,
        )
        .unwrap_or(2);
        let lines =
            crate::services::subscriptions::plan_change::calculate_components_mrr_with_slots(
                conn,
                tenant_id,
                subscription_id,
                &details.price_components,
                precision,
            )
            .await?;
        let mrr = self
            .restate_subscription_mrr(conn, &mut details, lines, precision, on)
            .await?;
        let delta = mrr - details.subscription.mrr_cents as i64;
        if delta != 0 {
            db(SubscriptionRow::update_subscription_mrr_delta(conn, subscription_id, delta).await)?;
            self.create_movement_mrr_log(conn, tenant_id, subscription_id, on, delta)
                .await?;
        }
        Ok(())
    }

    /// The subscription stepped at `issued_at`, a past moment, its documents settled then: the
    /// lifecycle replayed (sandbox seeding, imports of history). Returns the last document it
    /// wrote.
    pub(in crate::services) async fn replay_subscription_at_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        issued_at: DateTime<Utc>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<Option<DetailedInvoice>> {
        if issued_at > common_clock::now() {
            return Err(Report::new(StoreError::InvalidArgument(
                "a subscription period cannot be billed at a future moment".to_owned(),
            )));
        }
        let lock = db(SubscriptionRow::lock_customer_of(conn, subscription_id).await)?;
        let stepped = self
            .step_subscription_tx(
                conn,
                &Actor::System,
                tenant_id,
                subscription_id,
                issued_at,
                Issue::SettleAt(issued_at),
            )
            .await?;
        db(diesel_models::query::credit_inputs::bump_customer(conn, &lock).await)?;
        let mut invoices = stepped.map(|stepped| stepped.invoices).unwrap_or_default();
        for invoice in &invoices {
            delivery
                .apply_tx(conn, tenant_id, invoice.invoice.id)
                .await?;
        }
        Ok(invoices.pop())
    }
}

impl ServicesEdge {
    /// The subscription's lifecycle replayed up to `issued_at`, a past moment, each document due
    /// by then issued at it.
    pub async fn bill_subscription_period_at(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        issued_at: DateTime<Utc>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<Option<DetailedInvoice>> {
        let mut conn = self.store.get_conn().await?;
        self.services
            .transaction_reading_usage_first(&mut conn, |services, conn| {
                async move {
                    services
                        .replay_subscription_at_tx(
                            conn,
                            tenant_id,
                            subscription_id,
                            issued_at,
                            delivery,
                        )
                        .await
                }
                .scope_boxed()
            })
            .await
    }
}

/// The option a change written ahead moves the subscription to, once it applies. A switch to
/// `switched`'s version on its day by a change naming no option (one written before options were
/// recorded on it) moves to the option a change naming none picks, as of the day before, and a
/// version without options leaves none of the old version's.
async fn record_changed_option(
    store: &crate::Store,
    conn: &mut PgConn,
    row: &SubscriptionRow,
    change: &SubscriptionChangeRow,
    switched: Option<(PlanVersionId, NaiveDate)>,
) -> StoreResult<()> {
    let named = change
        .change
        .pointer("/ApplyPlanChange/new_plan_option_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|id| id.parse::<common_domain::ids::PlanOptionId>().ok());
    let option = match (named, switched) {
        (Some(option), _) => Some(option),
        (None, Some((to, on))) if to != row.plan_version_id => {
            let day_before = on.pred_opt().unwrap_or(on);
            let before = store
                .subscription_details_as_of(conn, row.tenant_id, row.id, day_before)
                .await?;
            crate::services::subscriptions::plan_change::unnamed_target_option(
                conn, store, &before, to,
            )
            .await?
            .map(|option| option.id)
        }
        (None, _) => return Ok(()),
    };
    // `row` is read once per step: a switch naming its option writes it, as a later switch of the
    // same step may have to undo the one before.
    if named.is_some() || option != row.plan_option_id {
        crate::services::subscriptions::currency::record_plan_option(conn, row.id, option).await?;
    }
    Ok(())
}

/// The plan change written ahead that a switch to `to` on `on` applies.
async fn written_ahead_change(
    conn: &mut PgConn,
    row: &SubscriptionRow,
    to: PlanVersionId,
    on: NaiveDate,
) -> StoreResult<Option<SubscriptionChangeRow>> {
    let mut changes = Vec::new();
    for kind in [
        SubscriptionChangeKindEnum::PlanChange,
        SubscriptionChangeKindEnum::Phase,
    ] {
        changes.extend(db(SubscriptionChangeRow::list_in_force(
            conn,
            &row.tenant_id,
            row.id,
            kind,
            on.and_time(chrono::NaiveTime::MIN),
        )
        .await)?);
    }
    Ok(changes.into_iter().find(|change| {
        change.effective_at.date() == on
            && change
                .change
                .pointer("/ApplyPlanChange/new_plan_version_id")
                .and_then(serde_json::Value::as_str)
                .and_then(|id| id.parse::<PlanVersionId>().ok())
                == Some(to)
    }))
}

/// Whether a switch of `transitions` applies a change written ahead and is still to record: its
/// settlement reads metering (R-077).
async fn settles_a_switch(
    conn: &mut PgConn,
    row: &SubscriptionRow,
    transitions: &[Transition],
) -> StoreResult<bool> {
    for transition in transitions {
        if let Transition::Plan { to, at, .. } = *transition {
            let on = at.date_naive();
            if !db(SubscriptionEventRow::switch_recorded(conn, row.id, to, on).await)?
                && written_ahead_change(conn, row, to, on).await?.is_some()
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Each amendment written ahead for a period's end, still to take effect, waits for the first
/// renewal on or after its day: a change since may have moved that end past it (R-106). A
/// timeline change keeps its day: one no longer on a renewal refuses what moved the renewals,
/// and one after the subscription's ending is left to its termination (R-146).
async fn defer_amendments_tx(
    conn: &mut PgConn,
    row: &SubscriptionRow,
    now: DateTime<Utc>,
) -> StoreResult<()> {
    let pending =
        db(SubscriptionChangeRow::list_pending(conn, &row.tenant_id, row.id, WRITTEN_AHEAD).await)?;
    if pending.is_empty() {
        return Ok(());
    }
    let loaded = load_facts(conn, row.tenant_id, row.id, now.date_naive(), None).await?;
    let settled_inside = db(SubscriptionChangeRow::settlements_due(conn, row.id).await)?;
    for change in &pending {
        if crate::services::subscriptions::phases::is_timeline_change(change) {
            if settled_inside.contains(&change.effective_at.date()) {
                continue;
            }
            timeline_change_on_renewal(change, &loaded.facts)?;
        } else {
            crate::services::facts::pending::defer_to_renewal_tx(conn, change, &loaded.facts, now)
                .await?;
        }
    }
    Ok(())
}

fn timeline_change_on_renewal(
    change: &SubscriptionChangeRow,
    facts: &billing_core::facts::Facts<crate::services::evaluate::pricer::StoreFee>,
) -> StoreResult<()> {
    let day = change.effective_at.date();
    // Written for its own day: a free timeline item from today, meant inside its period.
    if day <= change.created_at.date() {
        return Ok(());
    }
    let ends = facts
        .ending
        .map(|ending| ending.on())
        .into_iter()
        .chain(facts.end_date)
        .min();
    if ends.is_some_and(|ends| ends <= day) {
        return Ok(());
    }
    let at = day.and_time(chrono::NaiveTime::MIN).and_utc();
    if billing_core::evaluate::renewal_on_or_after(facts, day, at) == Some(day) {
        return Ok(());
    }
    Err(Report::new(StoreError::InvalidArgument(format!(
        "This would move the subscription's renewals off {day}, where a phase, dated item or \
         discount changes its terms: edit the subscription's timeline first"
    ))))
}

/// Every document of the subscription, one per slot (R-051, R-052): a written one is never due
/// again, refreshed while a draft, else corrected. An adjustment is the controller's only for a
/// change left to it; any other change settled its day in its own transaction (R-120).
async fn written_slots(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    evaluation: &billing_core::evaluate::Evaluation,
) -> StoreResult<Vec<DocumentSlot>> {
    let mut written: Vec<DocumentSlot> =
        db(InvoiceRow::written_slots_of(conn, tenant_id, subscription_id).await)?
            .into_iter()
            .filter_map(|(kind, on)| match kind {
                InvoiceKind::Recurring => Some(DocumentSlot::Recurring { period_start: on }),
                InvoiceKind::Final => Some(DocumentSlot::Final),
                InvoiceKind::Adjustment => Some(DocumentSlot::Adjustment { on }),
                InvoiceKind::OneOff | InvoiceKind::UsageThreshold => None,
            })
            .collect();
    let left = db(SubscriptionChangeRow::settlements_due(conn, subscription_id).await)?;
    written.extend(
        evaluation
            .charges
            .iter()
            .map(|charge| charge.document)
            .filter(|slot| matches!(slot, DocumentSlot::Adjustment { on } if !left.contains(on))),
    );
    Ok(written)
}

/// The latest day on or before `today` a fee row, a seat step or a minimum started or ended on:
/// the change an MRR movement belongs to.
fn latest_change<F>(facts: &Facts<F>, today: NaiveDate) -> Option<NaiveDate> {
    facts
        .fees
        .iter()
        .flat_map(|fee| [Some(fee.from), fee.to])
        .chain(facts.seats.iter().map(|seat| Some(seat.at)))
        .chain(
            facts
                .minimums
                .iter()
                .flat_map(|minimum| [Some(minimum.from), minimum.to]),
        )
        .flatten()
        .filter(|day| *day <= today)
        .max()
}

/// The changes written ahead for a renewal, applied by the step of their day: amendments and
/// phase boundaries, which write the same kind of rows (R-106, R-146).
const WRITTEN_AHEAD: &[SubscriptionChangeKindEnum] = &[
    SubscriptionChangeKindEnum::Amendment,
    SubscriptionChangeKindEnum::Phase,
];

async fn in_force_written_ahead(
    conn: &mut PgConn,
    row: &SubscriptionRow,
    at: DateTime<Utc>,
) -> StoreResult<Vec<SubscriptionChangeRow>> {
    db(SubscriptionChangeRow::list_in_force_unapplied(
        conn,
        &row.tenant_id,
        row.id,
        WRITTEN_AHEAD,
        at.naive_utc(),
    )
    .await)
}
