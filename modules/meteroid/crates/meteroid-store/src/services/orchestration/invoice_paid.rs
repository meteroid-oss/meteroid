use crate::StoreResult;
use crate::domain::customer_emails::EmailKind;
use crate::domain::outbox_event::InvoiceEvent;
use crate::repositories::InvoiceInterface;
use crate::repositories::PaymentRemindersInterface;
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::services::Services;
use crate::services::subscriptions::start::{StartTransition, mark_subscription_started_tx};
use crate::utils::periods::calculate_advance_period_range;
use common_domain::ids::TenantId;
use diesel_models::enums::CycleActionEnum;
use diesel_models::subscriptions::SubscriptionRow;
use scoped_futures::ScopedFutureExt;

impl Services {
    pub async fn on_invoice_paid(
        &self,
        event: InvoiceEvent,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let activated = self
            .activate_pending_slot_transactions(tenant_id, event.invoice_id, None)
            .await?;

        if !activated.is_empty() {
            tracing::info!(
                "Activated {} pending slot transactions for invoice {}",
                activated.len(),
                event.invoice_id
            );
            // TODO: Emit wh events regarding slot activations
        }

        // Activate subscription if needed (TrialExpired → Active on invoice paid)
        self.activate_subscription_on_invoice_paid(tenant_id, event.invoice_id)
            .await?;

        // Best effort: the sweep's unpayable scan ends whatever a failure here leaves behind.
        if let Err(e) = self
            .store
            .end_invoice_reminders(tenant_id, event.invoice_id)
            .await
        {
            log::error!(
                "Payment reminders of paid invoice {} (tenant {tenant_id}) not ended: {e:?}",
                event.invoice_id
            );
        }

        let receipt = self
            .store
            .last_settled_payment_tx_by_invoice_id(tenant_id, event.invoice_id)
            .await?;

        let invoice = self
            .store
            .get_invoice_by_id(tenant_id, event.invoice_id)
            .await?;

        if invoice.issued_at.is_some() || self.invoice_delivery_held(tenant_id, invoice.id).await? {
            return Ok(());
        }

        if invoice.pdf_document_id.is_none() {
            tracing::warn!("Invoice {} has no pdf document id", invoice.id);
            return Ok(());
        }
        if receipt.is_none() {
            tracing::warn!("No receipt found for invoice {}", event.invoice_id);
            return Ok(());
        }
        self.enqueue_invoice_email(tenant_id, invoice.id, EmailKind::InvoicePaid)
            .await
    }

    /// Activate subscription(s) when invoice is paid. A consolidated parent has no
    /// subscription_id of its own, so we activate each merged member.
    async fn activate_subscription_on_invoice_paid(
        &self,
        tenant_id: TenantId,
        invoice_id: common_domain::ids::InvoiceId,
    ) -> StoreResult<()> {
        let invoice = self.store.get_invoice_by_id(tenant_id, invoice_id).await?;

        let subscription_ids: Vec<_> = match invoice.subscription_id {
            Some(id) => vec![id],
            None => self
                .store
                .list_consolidated_children(tenant_id, invoice_id)
                .await?
                .into_iter()
                .filter_map(|child| child.subscription_id)
                .collect(),
        };

        for subscription_id in subscription_ids {
            self.activate_trial_expired_subscription(tenant_id, subscription_id)
                .await?;
        }

        Ok(())
    }

    /// Transition a single subscription from TrialExpired → Active (no-op for other states).
    async fn activate_trial_expired_subscription(
        &self,
        tenant_id: TenantId,
        subscription_id: common_domain::ids::SubscriptionId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                async move {
                    let lock = SubscriptionRow::lock_customer_of(conn, subscription_id).await?;
                    let subscription =
                        SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                            .await?;

                    if subscription.subscription.status
                        != diesel_models::enums::SubscriptionStatusEnum::TrialExpired
                    {
                        return Ok(());
                    }

                    let period_start = subscription.subscription.current_period_start;
                    let range = calculate_advance_period_range(
                        period_start,
                        subscription.subscription.billing_day_anchor as u32,
                        true,
                        &subscription.subscription.period.into(),
                    );

                    mark_subscription_started_tx(
                        conn,
                        &*self.store.hooks.credit,
                        tenant_id,
                        subscription_id,
                        StartTransition::TrialExpiredToActive {
                            current_period_start: range.start,
                            current_period_end: Some(range.end),
                            next_cycle_action: Some(CycleActionEnum::RenewSubscription),
                            cycle_index: Some(0),
                        },
                    )
                    .await?;
                    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;

                    tracing::info!(
                        "Activated subscription {} from TrialExpired on invoice paid",
                        subscription_id
                    );

                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }
}
