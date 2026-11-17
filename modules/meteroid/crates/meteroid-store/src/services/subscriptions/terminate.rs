use crate::StoreResult;
use crate::domain::SubscriptionDetails;
use crate::domain::entity_activity::Actor;
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::subscriptions::mrr::subscription_movement;
use crate::services::subscriptions::plan_change_credit::CancellationRecord;
use crate::store::PgConn;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::enums::{MrrMovementType, SubscriptionEventType};
use diesel_models::invoices::InvoiceRow;
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscription_facts::{ENDING_CANCEL, SubscriptionEndingRow};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use uuid::Uuid;

/// Whether terminations issue final invoices: the hours of an ending's day after its invoice was
/// issued are billed on one of its own (R-016).
const ISSUES_FINAL_INVOICES: bool = true;

/// Whether terminations issue final invoices.
pub(in crate::services) fn issues_final_invoices() -> bool {
    ISSUES_FINAL_INVOICES
}

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

/// Whether the recurring invoice of the day the subscription's cancellation ends takes no more fee
/// lines, so what the ending cuts goes on a final invoice of its own (R-016): written before the
/// cancellation was requested, or, for a backdated one, kept: any but a draft it closes.
pub(in crate::services) async fn ending_day_invoiced(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    endings: &[SubscriptionEndingRow],
) -> StoreResult<bool> {
    let Some(cancel) = endings.iter().find(|row| row.kind == ENDING_CANCEL) else {
        return Ok(false);
    };
    Ok(InvoiceRow::find_existing_recurring_invoice(
        conn,
        tenant_id,
        subscription_id,
        cancel.at.date(),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?
    .is_some_and(|invoice| {
        use diesel_models::enums::InvoiceStatusEnum;
        if cancel.is_backdated_cancellation() {
            // Written after the request (a voided period billed again), a draft is its own.
            !matches!(
                invoice.status,
                InvoiceStatusEnum::Draft | InvoiceStatusEnum::Closed
            ) || invoice.created_at >= cancel.requested_at
        } else {
            invoice.created_at < cancel.requested_at
        }
    }))
}

impl Services {
    /// Credits what a cancellation mid-period leaves unused: each fee's lines by their unused time
    /// (R-077), a shortfall charged up front, its period packs capped by their unused paid credit.
    /// Returns what to record for the period's drafts, credited once issued.
    pub(in crate::services) async fn issue_cancellation_refund(
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

            // Only an open period, or one a backdated cancellation reaches into, can leave unused
            // advance to refund: from its date, every period issued since comes back too (R-077).
            // Ending on the period's end still ends a longer fee's period, billed past it.
            let period_start = sub.current_period_start;
            let Some(period_end) = sub.current_period_end else {
                return Ok(None);
            };
            if termination_date > period_end {
                return Ok(None);
            }
            // A free trial has billed nothing in advance; a paid trial did, like any period.
            if sub.status == crate::domain::SubscriptionStatusEnum::TrialActive
                && super::trial_is_free_tx(conn, sub).await?
            {
                return Ok(None);
            }

            let notes = self
                .settle_cancelled_components_tx(conn, actor, details, termination_date, ended_at)
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

    pub(in crate::services) async fn create_churn_mrr_log(
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
                    created_at: common_clock::now().naive_utc(),
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
