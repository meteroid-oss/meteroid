//! The payment reminder sweeper: pass 1 executes due steps (≤ 1 per invoice
//! per run, cursor advance and enqueue in one `FOR UPDATE` tx), pass 2 enrols candidates per
//! entity with a keyset bounded by the ladder reach. Kill switches are holds, never burns.

use crate::StoreResult;
use crate::domain::customer_emails::{EmailContext, EmailKind};
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::{ReminderEnrolledVia, ReversalKind};
use crate::domain::payment_reminders::{PaymentReminderSequence, SendWindow};
use crate::domain::{Customer, Invoice};
use crate::errors::StoreError;
use crate::repositories::CustomerEmailsInterface;
use crate::services::customer_emails::SkipReason;
use crate::services::payment_reminders::audience::{compile, evaluate_audience};
use crate::services::payment_reminders::schedule::{
    EnrollmentPlan, ExecutionPlan, anchor_date, fire_time, plan_enrollment, plan_execution,
    plan_restamp, plan_restart, schedulable,
};
use crate::services::{Services, ServicesEdge};
use crate::store::{PgConn, Store, StoreInternal};
use chrono::{Duration, NaiveDateTime};
use common_domain::ids::{
    BaseId, InvoiceId, InvoicingEntityId, PaymentReminderSequenceId, TenantId,
};
use diesel_models::customers::CustomerRow;
use diesel_models::enums::{
    InvoicePaymentStatus, InvoiceStatusEnum, ReminderNotEnrolledReasonEnum, ReminderStateEnum,
    ReminderStopReasonEnum,
};
use diesel_models::invoices::{InvoiceLockRow, InvoiceRow};
use diesel_models::lock::CustomerLock;
use diesel_models::payment_reminders::{
    InvoicePaymentReminderRow, PaymentReminderSequenceRow, PaymentReminderStepRow,
};
use diesel_models::payments::PaymentTransactionRow;
use diesel_models::query::payment_reminders::{
    EntityReachRow, InvoiceReminderUpdates, ReminderCandidateRow, ScanCursor,
    end_unpayable_scheduled, last_customer_email_at, list_entity_reach, list_reminder_candidates,
    list_reminder_due,
};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use std::collections::{HashMap, HashSet};
use std::time::{Duration as StdDuration, Instant};

pub const ENROLL_BATCH: i64 = 500;
pub const MAX_SCANNED_PER_ENTITY_PER_RUN: usize = 5_000;
pub const EXECUTE_BATCH: i64 = 200;
/// Wall-clock budget of one run; a backlog continues on the next cron tick.
pub const RUN_BUDGET: StdDuration = StdDuration::from_secs(10 * 60);
/// Defer applied on a pending payment, a missing PDF or an execution error (quarantine).
pub const DEFER_HOURS: i64 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepStats {
    pub executed: usize,
    pub deferred: usize,
    pub completed: usize,
    pub ended: usize,
    pub enrolled: usize,
    pub not_enrolled: usize,
    pub quarantined: usize,
    /// Entities whose enrolment pass failed and was skipped for this run.
    pub entity_errors: usize,
    pub truncated: bool,
}

impl SweepStats {
    pub fn did_work(&self) -> bool {
        *self != Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecOutcome {
    Executed,
    Deferred,
    Completed,
    Ended,
    /// A precondition changed between the scan and the lock; nothing written.
    Skipped,
}

/// What resolution and planning need for one entity: its send window, the `payment_reminder`
/// toggle and its enabled live sequences in position order (disabled ones are absent).
pub(crate) struct EntityRules {
    pub tenant_id: TenantId,
    pub window: SendWindow,
    pub reminders_enabled: bool,
    pub sequences: Vec<PaymentReminderSequence>,
}

impl EntityRules {
    pub fn find(&self, id: PaymentReminderSequenceId) -> Option<&PaymentReminderSequence> {
        self.sequences.iter().find(|s| s.id == id)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Resolved<'a> {
    pub sequence: &'a PaymentReminderSequence,
    pub via: ReminderEnrolledVia,
}

pub(crate) fn offsets(sequence: &PaymentReminderSequence) -> Vec<i32> {
    sequence.steps.iter().map(|s| s.offset_days).collect()
}

pub(crate) fn is_payable(invoice: &InvoiceRow) -> bool {
    invoice.status == InvoiceStatusEnum::Finalized
        && invoice.amount_due > 0
        && matches!(
            invoice.payment_status,
            InvoicePaymentStatus::Unpaid
                | InvoicePaymentStatus::PartiallyPaid
                | InvoicePaymentStatus::Errored
        )
}

/// What a reopen does to the invoice's reminder ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReopenedReminders {
    Restart,
    /// The payer contested the charge: stopped as `PAYMENT_DISPUTED` for an operator to restart.
    HoldForOperator,
}

impl ReopenedReminders {
    pub(crate) fn after_reversal(kind: Option<ReversalKind>) -> Self {
        if kind.is_some_and(|k| k.restarts_reminders()) {
            Self::Restart
        } else {
            Self::HoldForOperator
        }
    }
}

/// Decided from persisted facts, never from a live resolver call.
pub(crate) fn is_reminder_eligible(invoice: &InvoiceRow) -> bool {
    invoice.payment_status == InvoicePaymentStatus::Errored
        || invoice.auto_charge_requested_at.is_none()
}

fn invoice_activity(
    activity_type: ActivityType,
    invoice: &InvoiceRow,
    metadata: serde_json::Value,
) -> Activity {
    let mut activity = Activity::new(activity_type, EntityType::Invoice, invoice.id.as_uuid())
        .with_metadata(metadata);
    activity.agg_customer_id = Some(invoice.customer_id);
    activity.agg_subscription_id = invoice.subscription_id;
    activity
}

/// Schedulable steps between the cursor and the executed one: catch-up skipped them.
fn skipped_offsets(
    steps: &[i32],
    cursor: Option<i32>,
    executed: i32,
    anchor: chrono::NaiveDate,
    finalized_at: NaiveDateTime,
    window: &SendWindow,
) -> Vec<i32> {
    steps
        .iter()
        .copied()
        .filter(|o| cursor.is_none_or(|c| *o > c) && *o < executed)
        .filter(|o| schedulable(fire_time(anchor, *o, window), finalized_at))
        .collect()
}

impl StoreInternal {
    pub(crate) async fn load_entity_rules(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> StoreResult<EntityRules> {
        let settings = self
            .get_email_settings_tx(conn, tenant_id, invoicing_entity_id)
            .await?;
        let rows: Vec<PaymentReminderSequenceRow> =
            PaymentReminderSequenceRow::list_by_entity(conn, tenant_id, invoicing_entity_id, false)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .filter(|r| r.enabled)
                .collect();
        let ids: Vec<PaymentReminderSequenceId> = rows.iter().map(|r| r.id).collect();
        let mut steps: HashMap<PaymentReminderSequenceId, Vec<PaymentReminderStepRow>> =
            HashMap::new();
        for step in PaymentReminderStepRow::list_by_sequence_ids(conn, tenant_id, &ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        {
            steps.entry(step.sequence_id).or_default().push(step);
        }
        let sequences = rows
            .into_iter()
            .map(|row| {
                let steps = steps.remove(&row.id).unwrap_or_default();
                PaymentReminderSequence::from_rows(row, steps)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(EntityRules {
            tenant_id,
            window: settings.send_window(),
            reminders_enabled: settings.is_enabled(EmailKind::PaymentReminder),
            sequences,
        })
    }

    /// A valid customer override wins, then the enabled sequences in position
    /// order (audiences evaluated in Postgres on the still-unassigned batch, the default takes
    /// the rest). An invoice absent from the result has no winner.
    pub(crate) async fn resolve_sequences<'a>(
        &self,
        conn: &mut PgConn,
        rules: &'a EntityRules,
        candidates: &[(InvoiceId, Option<PaymentReminderSequenceId>)],
    ) -> StoreResult<HashMap<InvoiceId, Resolved<'a>>> {
        let mut assigned: HashMap<InvoiceId, Resolved<'a>> = HashMap::new();
        let mut unassigned: Vec<InvoiceId> = Vec::new();
        for (invoice_id, override_id) in candidates {
            match override_id.and_then(|id| rules.find(id)) {
                Some(sequence) => {
                    assigned.insert(
                        *invoice_id,
                        Resolved {
                            sequence,
                            via: ReminderEnrolledVia::CustomerOverride,
                        },
                    );
                }
                None => unassigned.push(*invoice_id),
            }
        }
        for sequence in &rules.sequences {
            if unassigned.is_empty() {
                break;
            }
            match &sequence.audience {
                None => {
                    for invoice_id in unassigned.drain(..) {
                        assigned.insert(
                            invoice_id,
                            Resolved {
                                sequence,
                                via: ReminderEnrolledVia::Default,
                            },
                        );
                    }
                }
                Some(audience) => {
                    let compiled = compile(audience)?;
                    let matched: HashSet<InvoiceId> =
                        evaluate_audience(conn, rules.tenant_id, &unassigned, &compiled)
                            .await?
                            .into_iter()
                            .collect();
                    unassigned.retain(|id| !matched.contains(id));
                    for invoice_id in matched {
                        assigned.insert(
                            invoice_id,
                            Resolved {
                                sequence,
                                via: ReminderEnrolledVia::Audience,
                            },
                        );
                    }
                }
            }
        }
        Ok(assigned)
    }

    /// Generation bumped, stop/pause/completion/end cleared, the sequence re-resolved
    /// with the current rules and the cursor placed so the last elapsed step executes once at
    /// the next slot (never before `floor`). Runs under the caller's invoice and row locks.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn restart_reminders_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice: &InvoiceRow,
        existing: Option<&InvoicePaymentReminderRow>,
        floor: Option<i32>,
        actor: &Actor,
        now: NaiveDateTime,
    ) -> StoreResult<()> {
        let failed = |msg: &str| Report::new(StoreError::FailedPrecondition(msg.to_string()));
        let customer = CustomerRow::find_by_id(conn, &invoice.customer_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if customer.payment_reminders_opt_out {
            return Err(failed("The customer opted out of payment reminders"));
        }
        let (Some(due_at), Some(finalized_at)) = (invoice.due_at, invoice.finalized_at) else {
            return Err(failed("The invoice has no due date"));
        };
        let rules = self
            .load_entity_rules(conn, tenant_id, invoice.invoicing_entity_id)
            .await?;
        let resolved = self
            .resolve_sequences(
                conn,
                &rules,
                &[(invoice.id, customer.payment_reminder_sequence_id)],
            )
            .await?;
        let Some(winner) = resolved.get(&invoice.id) else {
            return Err(failed(
                "No enabled reminder sequence matches this invoice; disabled sequences are skipped",
            ));
        };
        let steps = offsets(winner.sequence);
        let anchor = anchor_date(due_at, finalized_at, &rules.window);
        let mut plan = plan_restart(&steps, anchor, finalized_at, now, &rules.window);
        if let Some(floor) = floor
            && plan.cursor.is_none_or(|c| c < floor)
        {
            plan = EnrollmentPlan {
                cursor: Some(floor),
                next_reminder_at: plan_restamp(
                    &steps,
                    anchor,
                    finalized_at,
                    Some(floor),
                    now,
                    &rules.window,
                ),
            };
        }
        let via = winner.via.into();
        match existing {
            Some(_) => InvoiceReminderUpdates::restart(
                conn,
                invoice.id,
                tenant_id,
                winner.sequence.id,
                via,
                plan.cursor,
                plan.next_reminder_at,
                now,
            )
            .await
            .map(|_| ()),
            None => InvoiceReminderUpdates::enroll(
                conn,
                invoice.id,
                tenant_id,
                winner.sequence.id,
                via,
                plan.cursor,
                plan.next_reminder_at,
                now,
            )
            .await
            .map(|_| ()),
        }
        .map_err(Into::<Report<StoreError>>::into)?;
        self.record_audit_tx(
            conn,
            tenant_id,
            actor,
            AuditInput::Activity(invoice_activity(
                ActivityType::PaymentReminderRestarted,
                invoice,
                serde_json::json!({
                    "sequence_id": winner.sequence.id.as_base62(),
                    "sequence_name": winner.sequence.name,
                    "via": winner.via,
                    "cursor": plan.cursor,
                    "next_reminder_at": plan.next_reminder_at,
                }),
            )),
        )
        .await
    }

    /// Reopen hook (claw-back, credit-note void, abandoned auto-charge), called inside the
    /// caller's money transaction. `hand_over`: automatic collection is over, so the auto-charge
    /// stamp is cleared and reminders own the invoice. Reminder failures never propagate.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn on_invoice_reopened(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        was_paid: bool,
        hand_over: bool,
        reminders: ReopenedReminders,
        now: NaiveDateTime,
    ) -> StoreResult<()> {
        let InvoiceLockRow { lock, mut invoice } =
            InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        if !is_payable(&invoice) {
            return Ok(());
        }
        if hand_over && invoice.auto_charge_requested_at.is_some() {
            InvoiceRow::set_auto_charge_requested(conn, invoice_id, tenant_id, None)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            invoice.auto_charge_requested_at = None;
        }
        // A savepoint: a reminder error rolls back only itself, never the money transaction.
        let result = self
            .transaction_with(conn, |conn| {
                let (invoice, lock) = (&invoice, &lock);
                async move {
                    match reminders {
                        ReopenedReminders::Restart => {
                            self.restart_reopened_reminders_tx(
                                conn,
                                lock,
                                invoice,
                                was_paid,
                                was_paid || hand_over,
                                now,
                            )
                            .await
                        }
                        ReopenedReminders::HoldForOperator => {
                            self.hold_reopened_reminders_tx(conn, lock, invoice, now)
                                .await
                        }
                    }
                }
                .scope_boxed()
            })
            .await;
        match result {
            Ok(()) => {}
            Err(e) if matches!(e.current_context(), StoreError::FailedPrecondition(_)) => {
                log::info!("Reopened invoice {invoice_id} not restarted: {e}");
            }
            Err(e) => log::error!(
                "Payment reminders of reopened invoice {invoice_id} (tenant {tenant_id}) not restarted: {e:?}"
            ),
        }
        Ok(())
    }

    /// Only a ladder the payment had ended restarts (`ENDED`, or `ENROLLED` before the paid event
    /// ran); in-flight rows and operator stops are left alone. A `reopened` invoice without a row
    /// enrols here, since the enrolment scan only looks back as far as the ladder reaches.
    async fn restart_reopened_reminders_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        invoice: &InvoiceRow,
        was_paid: bool,
        reopened: bool,
        now: NaiveDateTime,
    ) -> StoreResult<()> {
        let tenant_id = lock.tenant_id();
        if !is_reminder_eligible(invoice) || invoice.consolidated_into_invoice_id.is_some() {
            return Ok(());
        }
        let Some(row) =
            InvoicePaymentReminderRow::find_by_invoice_id_for_update(conn, lock, invoice.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        else {
            if !reopened {
                return Ok(());
            }
            return self
                .restart_reminders_tx(conn, tenant_id, invoice, None, None, &Actor::System, now)
                .await;
        };
        let ended = row.state == ReminderStateEnum::Ended;
        if row.state == ReminderStateEnum::NotEnrolled || !(ended || was_paid) {
            return Ok(());
        }
        if matches!(
            row.stop_reason,
            Some(ReminderStopReasonEnum::Operator | ReminderStopReasonEnum::PaymentDisputed)
        ) {
            if ended {
                InvoiceReminderUpdates::reopen_stopped(conn, invoice.id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
            }
            return Ok(());
        }
        // A still-running ladder never goes back before a step the customer already received.
        let floor = if ended { None } else { row.last_offset_days };
        self.restart_reminders_tx(
            conn,
            tenant_id,
            invoice,
            Some(&row),
            floor,
            &Actor::System,
            now,
        )
        .await
    }

    /// A contested claw-back: the ladder stops as `PAYMENT_DISPUTED` until an operator restarts
    /// it, so the customer is never chased while the charge is disputed. An earlier stop is kept.
    async fn hold_reopened_reminders_tx(
        &self,
        conn: &mut PgConn,
        lock: &CustomerLock,
        invoice: &InvoiceRow,
        now: NaiveDateTime,
    ) -> StoreResult<()> {
        let tenant_id = lock.tenant_id();
        if !is_reminder_eligible(invoice) || invoice.consolidated_into_invoice_id.is_some() {
            return Ok(());
        }
        let row = InvoicePaymentReminderRow::find_by_invoice_id_for_update(conn, lock, invoice.id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if let Some(row) = &row
            && row.state == ReminderStateEnum::Ended
            && row.stopped_at.is_some()
        {
            InvoiceReminderUpdates::reopen_stopped(conn, invoice.id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            return Ok(());
        }
        let held = InvoiceReminderUpdates::hold(
            conn,
            invoice.id,
            tenant_id,
            ReminderStopReasonEnum::PaymentDisputed,
            now,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        if held == 0 {
            return Ok(());
        }
        self.record_audit_tx(
            conn,
            tenant_id,
            &Actor::System,
            AuditInput::Activity(invoice_activity(
                ActivityType::PaymentReminderStopped,
                invoice,
                serde_json::json!({ "reason": "PAYMENT_DISPUTED" }),
            )),
        )
        .await
    }

    /// Paid / void / uncollectible hook: ends the ladder in a savepoint, so a failure never
    /// rolls back the caller; the sweep's unpayable scan ends whatever this missed.
    pub(crate) async fn end_reminders_best_effort(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        now: NaiveDateTime,
    ) {
        let result = self
            .transaction_with(conn, |conn| {
                async move {
                    InvoiceReminderUpdates::clear_schedule(conn, invoice_id, tenant_id, now)
                        .await
                        .map(|_| ())
                        .map_err(Into::<Report<StoreError>>::into)
                }
                .scope_boxed()
            })
            .await;
        if let Err(e) = result {
            log::error!(
                "Payment reminders of invoice {invoice_id} (tenant {tenant_id}) not ended: {e:?}"
            );
        }
    }

    /// A timezone / send-hour change re-derives the next fire of every in-flight
    /// invoice of the entity (an elapsed step lands on the next slot).
    pub(crate) async fn restamp_reminders_for_entity(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        window: SendWindow,
        now: NaiveDateTime,
    ) -> StoreResult<usize> {
        let rows = InvoicePaymentReminderRow::list_in_flight_for_entity(
            conn,
            tenant_id,
            invoicing_entity_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        if rows.is_empty() {
            return Ok(0);
        }
        let sequence_ids: Vec<PaymentReminderSequenceId> = rows
            .iter()
            .map(|r| PaymentReminderSequenceId::from(r.sequence_id))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let mut steps: HashMap<PaymentReminderSequenceId, Vec<i32>> = HashMap::new();
        for step in PaymentReminderStepRow::list_by_sequence_ids(conn, tenant_id, &sequence_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        {
            steps
                .entry(step.sequence_id)
                .or_default()
                .push(step.offset_days);
        }
        let mut restamped = 0;
        for row in rows {
            let Some(offsets) = steps.get(&PaymentReminderSequenceId::from(row.sequence_id)) else {
                continue;
            };
            let anchor = anchor_date(row.due_at, row.finalized_at, &window);
            let Some(next) = plan_restamp(
                offsets,
                anchor,
                row.finalized_at,
                row.last_offset_days,
                now,
                &window,
            ) else {
                continue;
            };
            InvoiceReminderUpdates::defer(conn, InvoiceId::from(row.invoice_id), tenant_id, next)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            restamped += 1;
        }
        Ok(restamped)
    }
}

/// The reminder-only send-time checks: the row must still carry the message's
/// sequence and generation (else `superseded`, manual sends included), the customer must not
/// have opted out, and a scheduled send also needs the row enrolled, not stopped, not paused.
pub(crate) async fn reminder_send_guard(
    store: &Store,
    tenant_id: TenantId,
    invoice: &Invoice,
    customer: &Customer,
    sequence_id: PaymentReminderSequenceId,
    generation: i32,
    manual: bool,
) -> StoreResult<Option<SkipReason>> {
    let mut conn = store.get_conn().await?;
    let Some(row) = InvoicePaymentReminderRow::find_by_invoice_id(&mut conn, tenant_id, invoice.id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
    else {
        return Ok(Some(SkipReason::Superseded));
    };
    if row.sequence_id != Some(sequence_id) || row.generation != generation {
        return Ok(Some(SkipReason::Superseded));
    }
    if customer.payment_reminders_opt_out {
        return Ok(Some(SkipReason::OptedOut));
    }
    if !manual {
        if row.state != ReminderStateEnum::Enrolled {
            return Ok(Some(SkipReason::Superseded));
        }
        if row.stopped_at.is_some() {
            return Ok(Some(SkipReason::Stopped));
        }
        let now = common_domain::clock::now().naive_utc();
        if row.paused_until.is_some_and(|until| until > now) {
            return Ok(Some(SkipReason::Paused));
        }
    }
    Ok(None)
}

impl ServicesEdge {
    /// One sweep with an explicit clock: due steps first, then enrolment.
    pub async fn run_payment_reminder_sweep(&self, now: NaiveDateTime) -> StoreResult<SweepStats> {
        self.services.run_payment_reminder_sweep(now).await
    }
}

impl Services {
    pub(crate) async fn run_payment_reminder_sweep(
        &self,
        now: NaiveDateTime,
    ) -> StoreResult<SweepStats> {
        let started = Instant::now();
        let mut stats = SweepStats::default();
        self.execute_due_reminders(now, started, &mut stats).await?;
        self.enroll_reminder_candidates(now, started, &mut stats)
            .await?;
        Ok(stats)
    }

    // ---- pass 1: due scan -------------------------------------------------------------------

    async fn execute_due_reminders(
        &self,
        now: NaiveDateTime,
        started: Instant,
        stats: &mut SweepStats,
    ) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;
        let ended = end_unpayable_scheduled(&mut conn, now)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        stats.ended += ended;

        let mut visited: HashSet<InvoiceId> = HashSet::new();
        let mut after: Option<ScanCursor> = None;
        loop {
            if started.elapsed() > RUN_BUDGET {
                stats.truncated = true;
                return Ok(());
            }
            let page = list_reminder_due(&mut conn, now, EXECUTE_BATCH, after)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            let Some(last) = page.last() else {
                return Ok(());
            };
            after = Some(ScanCursor {
                at: last.next_reminder_at,
                invoice_id: InvoiceId::from(last.invoice_id),
            });
            for row in &page {
                let invoice_id = InvoiceId::from(row.invoice_id);
                let tenant_id = TenantId::from(row.tenant_id);
                if !visited.insert(invoice_id) {
                    continue;
                }
                match self
                    .execute_reminder_invoice(tenant_id, invoice_id, now)
                    .await
                {
                    Ok(ExecOutcome::Executed) => stats.executed += 1,
                    Ok(ExecOutcome::Deferred) => stats.deferred += 1,
                    Ok(ExecOutcome::Completed) => stats.completed += 1,
                    Ok(ExecOutcome::Ended) => stats.ended += 1,
                    Ok(ExecOutcome::Skipped) => {}
                    Err(e) => {
                        log::warn!("Payment reminder for invoice {invoice_id} quarantined: {e:?}");
                        self.quarantine(tenant_id, invoice_id, now).await;
                        stats.quarantined += 1;
                    }
                }
            }
        }
    }

    /// Never nulls the cursor: the invoice is simply looked at again in an hour.
    async fn quarantine(&self, tenant_id: TenantId, invoice_id: InvoiceId, now: NaiveDateTime) {
        let result = async {
            let mut conn = self.store.get_conn().await?;
            InvoiceReminderUpdates::defer(
                &mut conn,
                invoice_id,
                tenant_id,
                now + Duration::hours(DEFER_HOURS),
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)
        }
        .await;
        if let Err(e) = result {
            log::error!("Could not quarantine payment reminder for invoice {invoice_id}: {e:?}");
        }
    }

    /// One transaction per invoice; every precondition is re-checked under the locks.
    async fn execute_reminder_invoice(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        now: NaiveDateTime,
    ) -> StoreResult<ExecOutcome> {
        let store = self.store.clone();
        self.store
            .transaction(|conn| {
                let store = &store;
                async move {
                    let InvoiceLockRow { lock, invoice } =
                        InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    let Some(row) = InvoicePaymentReminderRow::find_by_invoice_id_for_update(
                        conn, &lock, invoice_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    else {
                        return Ok(ExecOutcome::Skipped);
                    };
                    let due = row.state == ReminderStateEnum::Enrolled
                        && row.stopped_at.is_none()
                        && row.next_reminder_at.is_some_and(|at| at <= now)
                        && row.paused_until.is_none_or(|until| until <= now);
                    let Some(sequence_id) = row.sequence_id.filter(|_| due) else {
                        return Ok(ExecOutcome::Skipped);
                    };
                    let sequence = PaymentReminderSequenceRow::find_by_id(conn, tenant_id, sequence_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if !sequence.is_some_and(|s| s.enabled && s.archived_at.is_none()) {
                        return Ok(ExecOutcome::Skipped);
                    }
                    let customer = CustomerRow::find_by_id(conn, &invoice.customer_id, &tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if customer.payment_reminders_opt_out {
                        return Ok(ExecOutcome::Skipped);
                    }

                    let retry_at = now + Duration::hours(DEFER_HOURS);
                    if invoice.payment_status == InvoicePaymentStatus::Processing {
                        InvoiceReminderUpdates::defer(conn, invoice_id, tenant_id, retry_at)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        return Ok(ExecOutcome::Deferred);
                    }
                    let (payable, Some(due_at), Some(finalized_at)) =
                        (is_payable(&invoice), invoice.due_at, invoice.finalized_at)
                    else {
                        InvoiceReminderUpdates::clear_schedule(conn, invoice_id, tenant_id, now)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        return Ok(ExecOutcome::Ended);
                    };
                    if !payable {
                        InvoiceReminderUpdates::clear_schedule(conn, invoice_id, tenant_id, now)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        return Ok(ExecOutcome::Ended);
                    }
                    if invoice.pdf_document_id.is_none()
                        || PaymentTransactionRow::exists_live_for_invoice(conn, invoice_id, tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                    {
                        InvoiceReminderUpdates::defer(conn, invoice_id, tenant_id, retry_at)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        return Ok(ExecOutcome::Deferred);
                    }

                    let steps: Vec<i32> =
                        PaymentReminderStepRow::list_by_sequence_ids(conn, tenant_id, &[sequence_id])
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                            .into_iter()
                            .map(|s| s.offset_days)
                            .collect();
                    let window = store
                        .internal
                        .get_email_settings_tx(conn, tenant_id, invoice.invoicing_entity_id)
                        .await?
                        .send_window();
                    let last_email = last_customer_email_at(conn, tenant_id, invoice_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    let anchor = anchor_date(due_at, finalized_at, &window);
                    match plan_execution(
                        &steps,
                        anchor,
                        finalized_at,
                        row.last_offset_days,
                        now,
                        last_email,
                        &window,
                    ) {
                        ExecutionPlan::Complete => {
                            InvoiceReminderUpdates::complete(conn, invoice_id, tenant_id, now)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                            store
                                .internal
                                .record_audit_tx(
                                    conn,
                                    tenant_id,
                                    &Actor::System,
                                    AuditInput::Activity(invoice_activity(
                                        ActivityType::PaymentReminderCompleted,
                                        &invoice,
                                        serde_json::json!({
                                            "sequence_id": sequence_id.as_base62(),
                                            "last_offset_days": row.last_offset_days,
                                        }),
                                    )),
                                )
                                .await?;
                            Ok(ExecOutcome::Completed)
                        }
                        ExecutionPlan::Reschedule(at) => {
                            InvoiceReminderUpdates::defer(conn, invoice_id, tenant_id, at)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                            Ok(ExecOutcome::Deferred)
                        }
                        ExecutionPlan::ExecuteStep { offset_days, next } => {
                            InvoiceReminderUpdates::advance(
                                conn, invoice_id, tenant_id, offset_days, next, now,
                            )
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                            store
                                .enqueue_customer_email_tx(
                                    conn,
                                    tenant_id,
                                    EmailKind::PaymentReminder,
                                    EmailContext::PaymentReminder {
                                        invoice_id,
                                        sequence_id,
                                        offset_days,
                                        generation: row.generation,
                                        manual: false,
                                    },
                                    reminder_dedupe_key(invoice_id, row.generation, offset_days),
                                )
                                .await?;
                            store
                                .internal
                                .record_audit_tx(
                                    conn,
                                    tenant_id,
                                    &Actor::System,
                                    AuditInput::Activity(invoice_activity(
                                        ActivityType::PaymentReminderStepExecuted,
                                        &invoice,
                                        serde_json::json!({
                                            "sequence_id": sequence_id.as_base62(),
                                            "offset_days": offset_days,
                                            "generation": row.generation,
                                            "skipped_offsets": skipped_offsets(
                                                &steps, row.last_offset_days, offset_days, anchor, finalized_at, &window,
                                            ),
                                            "next_reminder_at": next,
                                        }),
                                    )),
                                )
                                .await?;
                            Ok(ExecOutcome::Executed)
                        }
                    }
                }
                .scope_boxed()
            })
            .await
    }

    // ---- pass 2: enrolment ------------------------------------------------------------------

    async fn enroll_reminder_candidates(
        &self,
        now: NaiveDateTime,
        started: Instant,
        stats: &mut SweepStats,
    ) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;
        let reach = list_entity_reach(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        for entity in reach {
            if started.elapsed() > RUN_BUDGET {
                stats.truncated = true;
                return Ok(());
            }
            // One entity's bad audience or transient error never blocks the entities after it.
            if let Err(e) = self.enroll_entity(&mut conn, &entity, now, stats).await {
                log::warn!(
                    "Reminder enrolment for entity {} skipped this run: {e:?}",
                    entity.invoicing_entity_id
                );
                stats.entity_errors += 1;
            }
        }
        Ok(())
    }

    /// Pages the entity's candidates from where the previous run stopped (a run that hits the
    /// scan cap leaves its keyset behind, so a backlog of unresolved invoices never starves
    /// the newer ones) and enrols each page.
    async fn enroll_entity(
        &self,
        conn: &mut PgConn,
        entity: &EntityReachRow,
        now: NaiveDateTime,
        stats: &mut SweepStats,
    ) -> StoreResult<()> {
        let tenant_id = TenantId::from(entity.tenant_id);
        let invoicing_entity_id = InvoicingEntityId::from(entity.invoicing_entity_id);
        let rules = self
            .store
            .internal
            .load_entity_rules(conn, tenant_id, invoicing_entity_id)
            .await?;
        if !rules.reminders_enabled || rules.sequences.is_empty() {
            return Ok(());
        }
        // 24 h enrolment window plus timezone slack beyond the ladder's reach.
        let anchor_bound = now - Duration::days(i64::from(entity.max_offset) + 2);
        let mut after = self.scan_resume_cursor(invoicing_entity_id);
        let mut scanned = 0usize;
        let resume = loop {
            if scanned >= MAX_SCANNED_PER_ENTITY_PER_RUN {
                stats.truncated = true;
                break after;
            }
            let page = list_reminder_candidates(
                conn,
                tenant_id,
                invoicing_entity_id,
                anchor_bound,
                after,
                ENROLL_BATCH,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
            let Some(last) = page.last() else {
                break None;
            };
            scanned += page.len();
            after = Some(ScanCursor {
                at: last.anchor,
                invoice_id: InvoiceId::from(last.invoice_id),
            });
            let exhausted = (page.len() as i64) < ENROLL_BATCH;
            self.enroll_batch(conn, &rules, now, page, stats).await?;
            if exhausted {
                break None;
            }
        };
        self.set_scan_resume_cursor(invoicing_entity_id, resume);
        Ok(())
    }

    fn scan_resume_cursor(&self, entity: InvoicingEntityId) -> Option<ScanCursor> {
        self.reminder_scan_resume
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&entity)
            .copied()
    }

    fn set_scan_resume_cursor(&self, entity: InvoicingEntityId, cursor: Option<ScanCursor>) {
        let mut map = self
            .reminder_scan_resume
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match cursor {
            Some(cursor) => {
                map.insert(entity, cursor);
            }
            None => {
                map.remove(&entity);
            }
        }
    }

    async fn enroll_batch(
        &self,
        conn: &mut PgConn,
        rules: &EntityRules,
        now: NaiveDateTime,
        rows: Vec<ReminderCandidateRow>,
        stats: &mut SweepStats,
    ) -> StoreResult<()> {
        let candidates: Vec<(InvoiceId, Option<PaymentReminderSequenceId>)> = rows
            .iter()
            .map(|r| {
                (
                    InvoiceId::from(r.invoice_id),
                    r.customer_override_id.map(PaymentReminderSequenceId::from),
                )
            })
            .collect();
        let resolved = self
            .store
            .internal
            .resolve_sequences(conn, rules, &candidates)
            .await?;
        for row in rows {
            let invoice_id = InvoiceId::from(row.invoice_id);
            let Some(winner) = resolved.get(&invoice_id) else {
                continue;
            };
            let steps = offsets(winner.sequence);
            let anchor = anchor_date(row.due_at, row.finalized_at, &rules.window);
            let plan = plan_enrollment(&steps, anchor, row.finalized_at, now, &rules.window);
            let skipped: Vec<i32> = plan
                .as_ref()
                .and_then(|p| p.cursor)
                .map(|cursor| {
                    steps
                        .iter()
                        .copied()
                        .filter(|o| *o <= cursor)
                        .filter(|o| {
                            schedulable(fire_time(anchor, *o, &rules.window), row.finalized_at)
                        })
                        .collect()
                })
                .unwrap_or_default();
            match self
                .write_enrollment(rules.tenant_id, invoice_id, *winner, skipped, plan, now)
                .await
            {
                Ok(Some(true)) => stats.enrolled += 1,
                Ok(Some(false)) => stats.not_enrolled += 1,
                Ok(None) => {}
                Err(e) => log::warn!("Reminder enrolment of invoice {invoice_id} failed: {e:?}"),
            }
        }
        Ok(())
    }

    /// Enrols one invoice, in one transaction. `Some(true)` enrolled, `Some(false)` stamped
    /// `LADDER_STALE`, `None` when a re-check under the locks failed (nothing written).
    async fn write_enrollment(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        winner: Resolved<'_>,
        skipped: Vec<i32>,
        plan: Option<EnrollmentPlan>,
        now: NaiveDateTime,
    ) -> StoreResult<Option<bool>> {
        let store = self.store.clone();
        let sequence_id = winner.sequence.id;
        let sequence_name = winner.sequence.name.clone();
        let via = winner.via;
        self.store
            .transaction(|conn| {
                let store = &store;
                let sequence_name = &sequence_name;
                let skipped = &skipped;
                async move {
                    // The sequence comes before the customer: an archive or an override holds it
                    // while it updates the customers and reminders that use it.
                    let sequence = PaymentReminderSequenceRow::find_live_for_share(
                        conn,
                        tenant_id,
                        sequence_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .invoice;
                    if !sequence.is_some_and(|s| s.enabled) {
                        return Ok(None);
                    }
                    let customer = CustomerRow::find_by_id(conn, &invoice.customer_id, &tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if customer.payment_reminders_opt_out
                        || !is_payable(&invoice)
                        || !is_reminder_eligible(&invoice)
                        || invoice.consolidated_into_invoice_id.is_some()
                        || invoice.pdf_document_id.is_none()
                        || invoice.due_at.is_none()
                        || invoice.finalized_at.is_none()
                    {
                        return Ok(None);
                    }
                    if InvoicePaymentReminderRow::find_by_invoice_id(conn, tenant_id, invoice_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .is_some()
                    {
                        return Ok(None);
                    }

                    let Some(plan) = plan else {
                        InvoiceReminderUpdates::stamp_not_enrolled(
                            conn,
                            invoice_id,
                            tenant_id,
                            ReminderNotEnrolledReasonEnum::LadderStale,
                            now,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                        store
                            .internal
                            .record_audit_tx(
                                conn,
                                tenant_id,
                                &Actor::System,
                                AuditInput::Activity(invoice_activity(
                                    ActivityType::PaymentReminderNotEnrolled,
                                    &invoice,
                                    serde_json::json!({
                                        "reason": "LADDER_STALE",
                                        "sequence_id": sequence_id.as_base62(),
                                    }),
                                )),
                            )
                            .await?;
                        return Ok(Some(false));
                    };
                    InvoiceReminderUpdates::enroll(
                        conn,
                        invoice_id,
                        tenant_id,
                        sequence_id,
                        via.into(),
                        plan.cursor,
                        plan.next_reminder_at,
                        now,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    store
                        .internal
                        .record_audit_tx(
                            conn,
                            tenant_id,
                            &Actor::System,
                            AuditInput::Activity(invoice_activity(
                                ActivityType::PaymentReminderEnrolled,
                                &invoice,
                                serde_json::json!({
                                    "sequence_id": sequence_id.as_base62(),
                                    "sequence_name": sequence_name,
                                    "via": via,
                                    "cursor": plan.cursor,
                                    "next_reminder_at": plan.next_reminder_at,
                                    "skipped_offsets": skipped,
                                }),
                            )),
                        )
                        .await?;
                    Ok(Some(true))
                }
                .scope_boxed()
            })
            .await
    }
}

pub fn reminder_dedupe_key(invoice_id: InvoiceId, generation: i32, offset_days: i32) -> String {
    format!("payment_reminder:{invoice_id}:{generation}:{offset_days}")
}

pub fn manual_reminder_dedupe_key(invoice_id: InvoiceId, request_id: &str) -> String {
    format!("payment_reminder_manual:{invoice_id}:{request_id}")
}
