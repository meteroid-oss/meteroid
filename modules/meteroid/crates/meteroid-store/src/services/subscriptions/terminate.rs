use crate::StoreResult;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::subscription_changes::{ProrationResult, RemovedComponent};
use crate::domain::{SubscriptionAddOn, SubscriptionDetails};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::entity_activity::EntityActivityInterface;
use crate::services::credit::scope::ends_credit;
use crate::services::credit::subscription_end::end_subscription_credit_tx;
use crate::services::subscriptions::mrr::subscription_movement;
use crate::services::subscriptions::plan_change_credit::CancellationRecord;
use crate::services::subscriptions::proration::{calculate_proration_by, subscription_stub};
use crate::services::subscriptions::utils::scale_fee;
use crate::services::{InvoiceBillingMode, Services};
use crate::store::PgConn;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use common_domain::ids::{BaseId, SubscriptionId, TenantId};
use diesel_models::enums::{MrrMovementType, SubscriptionEventType, SubscriptionStatusEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::scheduled_events::ScheduledEventRow;
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::{SubscriptionCycleRowPatch, SubscriptionRow};
use error_stack::Report;
use uuid::Uuid;

/// Whether terminations issue final invoices. Off in the release that teaches every reader the
/// final invoice (so a binary rolled back to it never bills a final window twice), on after it.
const ISSUES_FINAL_INVOICES: bool = false;

/// Whether an immediate cancellation at `usage_cutoff` left usage to bill on `on`, after midnight.
fn final_window_open(on: NaiveDate, usage_cutoff: Option<NaiveDateTime>) -> bool {
    ISSUES_FINAL_INVOICES
        && usage_cutoff
            .is_some_and(|cutoff| cutoff.date() == on && cutoff > on.and_time(NaiveTime::MIN))
}

/// Whether an immediate cancellation on `on` bills what the period consumed until then on a final
/// invoice of its own: the period opened that day was invoiced already (its renewal, or a first
/// invoice billing in advance), and that invoice holds the period's key.
pub(in crate::services) async fn ends_with_final_invoice(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    on: NaiveDate,
    usage_cutoff: Option<NaiveDateTime>,
) -> StoreResult<bool> {
    if !final_window_open(on, usage_cutoff) {
        return Ok(false);
    }
    Ok(Box::pin(InvoiceRow::find_existing_recurring_invoice(
        conn,
        tenant_id,
        subscription_id,
        on,
    ))
    .await
    .map_err(Into::<Report<StoreError>>::into)?
    .is_some())
}

impl Services {
    /// `usage_cutoff` is the exact termination instant for an immediate cancellation; the terminal
    /// period bills usage up to it. `None` (date-effective cancel, pause, natural completion) settles
    /// at `date`'s midnight, preserving the date-granular period boundary.
    pub(in crate::services) async fn terminate_subscription(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        date: NaiveDate,
        terminate_with_state: SubscriptionStatusEnum,
        usage_cutoff: Option<NaiveDateTime>,
    ) -> StoreResult<bool> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let lock = SubscriptionRow::lock_customer_of(conn, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            // Never re-terminate an already-terminal subscription: a late scheduled cancellation must
            // not corrupt a subscription that already Completed/Aborted, nor double-book churn MRR.
            // Returns whether a transition actually happened, so callers only emit a webhook on a real
            // change (not on this idempotent no-op).
            let current_status = SubscriptionRow::get_status(conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            if current_status.is_terminal() {
                log::info!(
                    "Skipping termination of subscription {}: already in terminal state {:?}",
                    subscription_id,
                    current_status
                );
                return Ok(false);
            }

            // Read while the period is still the one the earlier release may have left.
            let before = SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            crate::services::invoice_lines::cadence::record_cadence_origins_tx(
                conn,
                &lock,
                crate::services::invoice_lines::cadence::CadenceSubject::of_row(
                    &before.subscription,
                ),
                date,
            )
            .await?;

            // Snapshot the subscription (with its still-current billing period, components
            // and minimum) before the patch below opens the period — the cancellation
            // refund prorates against this original window.
            let original_details: Option<SubscriptionDetails> =
                if terminate_with_state == SubscriptionStatusEnum::Cancelled {
                    Some(
                        self.store
                            .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                            .await?,
                    )
                } else {
                    None
                };

            // Capture before the patch so the pending-checkout guard below doesn't depend on the
            // patch leaving the column untouched; reuse the snapshot on the cancellation path.
            let pending_checkout = match &original_details {
                Some(details) => details.subscription.pending_checkout,
                None => {
                    SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .subscription
                        .pending_checkout
                }
            };

            // While the period is still open, as an immediate plan change before it rotates the
            // plan: the packs settle the usage up to the end, then the final invoice takes that
            // over and draws the rest, whether it is issued now or after its grace period.
            let ended_at = usage_cutoff.map_or_else(
                || date.and_time(chrono::NaiveTime::MIN).and_utc(),
                |cutoff| cutoff.and_utc(),
            );
            let settling = match &original_details {
                Some(details) => {
                    self.issue_cancellation_refund(conn, actor, details, date, ended_at)
                        .await?
                }
                None => None,
            };

            // Persist the settlement instant only for an immediate cancel (the sole caller that supplies
            // a sub-day cutoff); the terminal period then bills usage up to it. Date-effective cancels,
            // pause, and natural completion leave it unchanged (null) and keep the date-granular midnight
            // period boundary — so the column also stays a clean "immediate-termination instant" marker.
            let terminated_at = usage_cutoff.map(Some);

            let patch = SubscriptionCycleRowPatch {
                id: subscription_id,
                tenant_id,
                status: Some(terminate_with_state.clone()),
                next_cycle_action: Some(None),
                current_period_start: Some(date),
                current_period_end: Some(None),
                cycle_index: None, // we don't increase the cycle index on termination
                pending_checkout: None,
                processing_started_at: None, // no need to clear on termination
                billing_start_date: None,
                billing_day_anchor: None,
                terminated_at,
            };

            patch.patch(conn).await?;

            // Cancel all pending subscription-level events (lifecycle + EndTrial).
            // Billing events (FinalizeInvoice, RetryPayment) are preserved.
            let cancelled_count = ScheduledEventRow::cancel_pending_subscription_events(
                conn,
                subscription_id,
                &tenant_id,
                &format!(
                    "Subscription terminated with status {:?}",
                    terminate_with_state
                ),
            )
            .await
            .map_err(Into::<error_stack::Report<StoreError>>::into)?;

            if cancelled_count > 0 {
                log::info!(
                    "Cancelled {} pending lifecycle events for terminated subscription {}",
                    cancelled_count,
                    subscription_id,
                );
            }

            // Before the final invoice, so its settlement already sees the credit lapsing.
            if ends_credit(&terminate_with_state) {
                end_subscription_credit_tx(
                    conn,
                    &*self.store.hooks.credit,
                    tenant_id,
                    subscription_id,
                    ended_at,
                    common_domain::clock::now(),
                )
                .await?;
            }

            // A subscription still pending checkout was never activated: no active period to bill, so
            // skip billing — cancelling it must not create (and finalize) an invoice. Mirrors the
            // creation-time guard on `pending_checkout`.
            // A period already billed (or voided) is not billed again, so no invoice books the churn.
            let period_already_billed =
                InvoiceRow::find_existing_recurring_invoice(conn, tenant_id, subscription_id, date)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .is_some();
            // Ended before its start date: no period ever opened, nothing to bill.
            let never_started = current_status == SubscriptionStatusEnum::PendingActivation;
            let invoice_created = if pending_checkout || never_started {
                false
            } else {
                // The period opened today was invoiced already and holds its key: what it consumed
                // until now goes on a final invoice of its own. First, so the period's own invoice,
                // billed below if still a draft, no longer reads its usage past the period's start.
                if period_already_billed && final_window_open(date, usage_cutoff) {
                    self.bill_final_invoice_tx(conn, actor, tenant_id, subscription_id)
                        .await?;
                }
                let billed = self
                    .bill_subscription_tx(
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        InvoiceBillingMode::AwaitGracePeriodIfApplicable,
                    )
                    .await?;
                billed.is_some() && !period_already_billed
            };

            // Create churn MRR movement log only when no invoice was created.
            // When an invoice IS created, `process_mrr` (called during invoice insertion)
            // already handles the MRR delta and movement logs for the Cancelled event.
            if !invoice_created
                && terminate_with_state == SubscriptionStatusEnum::Cancelled
                && let Err(e) = self
                    .create_churn_mrr_log(conn, tenant_id, subscription_id, date)
                    .await
            {
                log::error!(
                    "Failed to create churn MRR log for subscription {}: {:?}",
                    subscription_id,
                    e
                );
            }

            // Last, as the churn log above: the CANCELLED event is locked no earlier than before.
            if let Some((record, notes)) = settling {
                self.record_settling_cancellation_tx(conn, subscription_id, date, &record, &notes)
                    .await?;
            }

            // Attribute the terminal transition to the actor that scheduled it (propagated
            // from the scheduled event); System when it was an internal/automated trigger.
            let activity_type = match terminate_with_state {
                SubscriptionStatusEnum::Cancelled => Some(ActivityType::SubscriptionCancelled),
                SubscriptionStatusEnum::Paused => Some(ActivityType::SubscriptionPaused),
                _ => None,
            };
            if let Some(activity_type) = activity_type {
                let customer_id =
                    SubscriptionRow::get_customer_id(conn, &tenant_id, subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                let activity = Activity::new(
                    activity_type,
                    EntityType::Subscription,
                    subscription_id.as_uuid(),
                )
                .agg_customer(customer_id);
                self.store
                    .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;
            }

            Ok(true)
        })
        .await
    }

    /// Credits the unused, advance-billed part of the period of a subscription cancelled
    /// mid-period, as an immediate plan change does: notes against the invoices that billed its
    /// fees, a shortfall charged up front before the true-up, and its period packs, each pack
    /// capped by its unused paid credit
    /// once it paid the usage up to `ended_at`. Fees prorate by whole days (the termination day
    /// counts as unused) while usage runs to the exact instant. Returns what to record for the
    /// period's invoices still drafts, credited once issued.
    async fn issue_cancellation_refund(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        details: &SubscriptionDetails,
        termination_date: NaiveDate,
        ended_at: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<Option<(CancellationRecord, Vec<common_domain::ids::CreditNoteId>)>> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let sub = &details.subscription;

            // Only an open mid-period cancellation can leave unused advance to refund.
            let period_start = sub.current_period_start;
            let Some(period_end) = sub.current_period_end else {
                return Ok(None);
            };
            // On the period's first day nothing of it was used: the whole period comes back.
            if termination_date < period_start || termination_date >= period_end {
                return Ok(None);
            }
            // A free trial has billed nothing in advance; a paid trial did, like any period.
            if sub.status == crate::domain::SubscriptionStatusEnum::TrialActive
                && super::trial_is_free_tx(conn, sub).await?
            {
                return Ok(None);
            }

            let precision = crate::constants::Currencies::resolve_currency_precision(&sub.currency)
                .unwrap_or(2);
            let proration = cancellation_proration(
                details,
                period_start,
                period_end,
                termination_date,
                precision,
            );
            debug_assert!(proration.lines.iter().all(|line| line.amount_cents <= 0));
            let notes = self
                .settle_cancelled_components_tx(
                    conn,
                    actor,
                    details,
                    &proration,
                    termination_date,
                    ended_at,
                )
                .await?;
            // An invoice of the period still a draft is credited the same way once issued.
            Ok(Some((
                CancellationRecord {
                    settle_at: ended_at,
                    period_start,
                    period_end,
                },
                notes,
            )))
        })
        .await
    }

    async fn create_churn_mrr_log(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        termination_date: NaiveDate,
    ) -> StoreResult<()> {
        // Fetch the cancellation subscription event
        let event = SubscriptionEventRow::fetch_by_subscription_id_and_event_type(
            conn,
            subscription_id,
            SubscriptionEventType::Cancelled,
            termination_date,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let event = match event {
            Some(e) => e,
            None => {
                log::warn!(
                    "No cancellation event found for subscription {} at date {}. Skipping churn MRR log.",
                    subscription_id,
                    termination_date
                );
                return Ok(());
            }
        };

        // Skip if already processed (idempotency)
        if event.bi_mrr_movement_log_id.is_some() {
            log::info!(
                "Churn MRR log already exists for subscription {}. Skipping.",
                subscription_id
            );
            return Ok(());
        }

        let mrr_delta = match event.mrr_delta {
            Some(delta) if delta != 0 => delta,
            _ => {
                log::info!(
                    "No MRR delta for cancellation event of subscription {}. Skipping churn MRR log.",
                    subscription_id
                );
                return Ok(());
            }
        };

        // Get subscription details for currency and plan_version_id
        let subscription =
            SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let mrr_log = subscription_movement(
            &subscription.subscription,
            MrrMovementType::Churn,
            "Subscription cancelled",
            mrr_delta,
            termination_date,
        );

        // The movement log is the transactional source the analytics projector
        // reads into ClickHouse `fact_mrr_movement`; no Postgres rollup remains.
        let inserted_log = mrr_log
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Update the subscription event to link to the MRR log (for audit trail)
        SubscriptionEventRow::update_mrr_movement_log_id(conn, event.id, inserted_log.id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Update subscription MRR
        SubscriptionRow::update_subscription_mrr_delta(conn, subscription_id, mrr_delta)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        log::info!(
            "Created churn MRR log for subscription {}: {} cents",
            subscription_id,
            mrr_delta
        );

        Ok(())
    }

    /// Churn the MRR of a subscription that completed naturally (reached its fixed end
    /// date and moved to `Completed`). Unlike a cancellation — which is scheduled and
    /// already carries a `Cancelled` event — natural completion emits no event, so this
    /// records the terminal `Ended` event (flipping `is_active` off) and books the
    /// negative Churn movement directly. The event keeps `mrr_delta: None` so
    /// `process_mrr` never double-books it. Idempotent per completion date.
    pub(in crate::services) async fn churn_ended_subscription(
        &self,
        conn: &mut PgConn,
        subscription: &SubscriptionRow,
        completion_date: NaiveDate,
    ) -> StoreResult<()> {
        let subscription_id = subscription.id;

        // Emit the terminal lifecycle event once; a cycle re-run finds the existing one.
        let event = match SubscriptionEventRow::fetch_by_subscription_id_and_event_type(
            conn,
            subscription_id,
            SubscriptionEventType::Ended,
            completion_date,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        {
            Some(existing) => existing,
            None => {
                let event = SubscriptionEventRow {
                    id: Uuid::now_v7(),
                    mrr_delta: None,
                    event_type: SubscriptionEventType::Ended,
                    created_at: common_domain::clock::now().naive_utc(),
                    applies_to: completion_date,
                    subscription_id,
                    bi_mrr_movement_log_id: None,
                    details: None,
                };
                event
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
            }
        };

        // Already booked (idempotency) or there is nothing to churn.
        if event.bi_mrr_movement_log_id.is_some() || subscription.mrr_cents == 0 {
            return Ok(());
        }

        let churn = -subscription.mrr_cents;
        let inserted_log = subscription_movement(
            subscription,
            MrrMovementType::Churn,
            "Subscription ended",
            churn,
            completion_date,
        )
        .insert(conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        SubscriptionEventRow::update_mrr_movement_log_id(conn, event.id, inserted_log.id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        SubscriptionRow::update_subscription_mrr_delta(conn, subscription_id, churn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(())
    }

    /// Record an Expansion (delta > 0) or Contraction (delta < 0) MRR movement for an
    /// in-place subscription change (add-on or plan amendment). Writes the
    /// `bi_mrr_movement_log` row the analytics projector reads into ClickHouse
    /// `fact_mrr_movement`. The originating `Switch` event keeps `mrr_delta: None`,
    /// so `process_mrr` never books it — this is the sole writer of the movement,
    /// which avoids a double count.
    pub(in crate::services) async fn create_movement_mrr_log(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        applies_to: NaiveDate,
        mrr_delta: i64,
    ) -> StoreResult<()> {
        if mrr_delta == 0 {
            return Ok(());
        }
        let is_expansion = mrr_delta > 0;
        let movement_type = if is_expansion {
            MrrMovementType::Expansion
        } else {
            MrrMovementType::Contraction
        };

        let subscription =
            SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        subscription_movement(
            &subscription.subscription,
            movement_type,
            if is_expansion {
                "Subscription expansion"
            } else {
                "Subscription contraction"
            },
            mrr_delta,
            applies_to,
        )
        .insert(conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        log::info!(
            "Created {} MRR log for subscription {subscription_id}: {mrr_delta} cents",
            if is_expansion {
                "expansion"
            } else {
                "contraction"
            }
        );
        Ok(())
    }
}

/// The unused part, from `termination_date`, of the period `[period_start, period_end)` that the
/// subscription's components and add-ons billed in advance: what its cancellation credits.
pub(crate) fn cancellation_proration(
    details: &SubscriptionDetails,
    period_start: NaiveDate,
    period_end: NaiveDate,
    termination_date: NaiveDate,
    precision: u8,
) -> ProrationResult {
    // Keyed by lineage, so the credit matches the line that originally billed it in advance.
    let mut removed: Vec<RemovedComponent> = Vec::new();
    for c in &details.price_components {
        removed.push(RemovedComponent {
            name: c.name.clone(),
            current_fee: c.fee.clone(),
            current_period: c.period,
            net_key: Some(c.lineage().to_string()),
        });
    }
    removed.extend(details.add_ons.iter().map(removed_add_on));
    calculate_proration_by(
        &[],
        &[],
        &removed,
        (period_start, period_end),
        termination_date,
        precision,
        subscription_stub(
            &details.subscription,
            (period_start, period_end),
            termination_date,
        ),
    )
}

/// An ending add-on as the proration credits it: its fee for every instance, keyed by lineage.
pub(crate) fn removed_add_on(add_on: &SubscriptionAddOn) -> RemovedComponent {
    RemovedComponent {
        name: add_on.name.clone(),
        current_fee: scale_fee(&add_on.fee, add_on.quantity),
        current_period: add_on.period,
        net_key: Some(add_on.lineage().to_string()),
    }
}
