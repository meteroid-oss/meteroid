use crate::StoreResult;
use crate::errors::StoreError;
use crate::services::{InvoiceBillingMode, Services};
use crate::store::PgConn;
use chrono::NaiveDate;
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::bi::BiMrrMovementLogRowNew;
use diesel_models::enums::{MrrMovementType, SubscriptionEventType, SubscriptionStatusEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::{SubscriptionCycleRowPatch, SubscriptionRow};
use error_stack::Report;
use uuid::Uuid;

impl Services {
    pub(in crate::services) async fn terminate_subscription(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        date: NaiveDate,
        terminate_with_state: SubscriptionStatusEnum,
    ) -> StoreResult<()> {
        let patch = SubscriptionCycleRowPatch {
            id: subscription_id,
            tenant_id,
            status: Some(terminate_with_state.clone()),
            next_cycle_action: Some(None),
            current_period_start: Some(date),
            current_period_end: Some(None),
            cycle_index: None, // we don't increase the cycle index on termination
            pending_checkout: None,
        };

        patch.patch(conn).await?;

        self.bill_subscription_tx(
            conn,
            tenant_id,
            subscription_id,
            InvoiceBillingMode::AwaitGracePeriodIfApplicable,
        )
        .await?;

        // Create churn MRR movement log for cancellations (non-blocking to avoid failing cancellations)
        if terminate_with_state == SubscriptionStatusEnum::Cancelled
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

        Ok(())
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

        // Find the last finalized invoice to link the MRR log to
        let last_invoice =
            InvoiceRow::find_last_by_subscription_id(conn, tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let invoice_id = match last_invoice {
            Some(inv) => inv.id,
            None => {
                log::warn!(
                    "No finalized invoice found for subscription {}. Cannot create churn MRR log.",
                    subscription_id
                );
                return Ok(());
            }
        };

        // Create the churn MRR movement log
        let mrr_log = BiMrrMovementLogRowNew {
            id: Uuid::now_v7(),
            description: "Subscription cancelled".to_string(),
            movement_type: MrrMovementType::Churn,
            net_mrr_change: mrr_delta,
            currency: subscription.subscription.currency.clone(),
            applies_to: termination_date,
            invoice_id,
            credit_note_id: None,
            plan_version_id: subscription.subscription.plan_version_id,
            tenant_id,
        };

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
}
