use crate::StoreResult;
use crate::domain::outbox_event::InvoiceEvent;
use crate::domain::pgmq::{PgmqMessageNew, PgmqQueue, SendEmailRequest};
use crate::errors::StoreError;
use crate::repositories::InvoiceInterface;
use crate::repositories::customers::CustomersInterfaceAuto;
use crate::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::repositories::pgmq::PgmqInterface;
use crate::services::Services;
use crate::utils::periods::calculate_advance_period_range;
use common_domain::ids::TenantId;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::enums::CycleActionEnum;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;

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

        let receipt = self
            .store
            .last_settled_payment_tx_by_invoice_id(tenant_id, event.invoice_id)
            .await?;

        let invoice = self
            .store
            .get_invoice_by_id(tenant_id, event.invoice_id)
            .await?;

        if invoice.issued_at.is_some() {
            return Ok(());
        }

        let invoice_pdf_id = if let Some(id) = invoice.pdf_document_id {
            id
        } else {
            tracing::warn!("Invoice {} has no pdf document id", invoice.id);
            return Ok(());
        };

        let receipt = if let Some(receipt) = receipt {
            receipt
        } else {
            tracing::warn!("No receipt found for invoice {}", event.invoice_id);
            return Ok(());
        };

        let customer = self
            .store
            .find_customer_by_id(invoice.customer_id, tenant_id)
            .await?;

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(customer.invoicing_entity_id))
            .await?;

        let label = invoice
            .plan_name
            .as_ref()
            .map(|plan| format!("Your {} invoice was paid successfully.", plan))
            .unwrap_or_else(|| "Invoice for services was paid successfully.".to_string());

        let event: StoreResult<PgmqMessageNew> = SendEmailRequest::InvoicePaid {
            tenant_id,
            invoice_id: invoice.id,
            invoice_number: invoice.invoice_number,
            invoicing_entity_id: invoicing_entity.id,
            invoice_date: invoice.invoice_date,
            invoice_due_date: invoice.due_at.map_or(invoice.invoice_date, |d| d.date()),
            label,
            amount_paid: receipt.amount,
            currency: invoice.currency,
            company_name: invoice.seller_details.legal_name.clone(),
            logo_attachment_id: invoicing_entity.logo_attachment_id,
            invoicing_emails: customer.invoicing_emails,
            invoice_pdf_id,
            receipt_pdf_id: receipt.receipt_pdf_id,
        }
        .try_into();

        self.store
            .pgmq_send_batch(PgmqQueue::SendEmailRequest, vec![event?])
            .await?;

        Ok(())
    }

    /// Activate subscription when invoice is paid.
    /// Handles TrialExpired → Active transition for OnCheckout subscriptions.
    async fn activate_subscription_on_invoice_paid(
        &self,
        tenant_id: TenantId,
        invoice_id: common_domain::ids::InvoiceId,
    ) -> StoreResult<()> {
        let invoice = self.store.get_invoice_by_id(tenant_id, invoice_id).await?;

        let subscription_id = match invoice.subscription_id {
            Some(id) => id,
            None => return Ok(()), // No subscription, nothing to activate
        };

        self.store
            .transaction(|conn| {
                async move {
                    let subscription =
                        SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                            .await?;

                    // Only activate if TrialExpired (waiting for payment to activate)
                    if subscription.subscription.status
                        != diesel_models::enums::SubscriptionStatusEnum::TrialExpired
                    {
                        return Ok(());
                    }

                    // Trial expired subscription paid - transition to Active
                    let period_start = subscription.subscription.current_period_start;

                    let range = calculate_advance_period_range(
                        period_start,
                        subscription.subscription.billing_day_anchor as u32,
                        true,
                        &subscription.subscription.period.into(),
                    );

                    SubscriptionRow::transition_trial_expired_to_active(
                        conn,
                        &subscription_id,
                        &tenant_id,
                        range.start,
                        Some(range.end),
                        Some(CycleActionEnum::RenewSubscription),
                        Some(0),
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

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
