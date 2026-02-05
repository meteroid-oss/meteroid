use crate::StoreResult;
use crate::domain::outbox_event::{InvoicePdfGeneratedEvent, OutboxEvent};
use crate::domain::pgmq::{PaymentRequestEvent, PgmqMessageNew, PgmqQueue, SendEmailRequest};
use crate::domain::{
    Customer, Invoice, InvoicePaymentStatus, InvoicingEntity, ResolvedPaymentMethod,
};
use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use crate::repositories::customers::CustomersInterfaceAuto;
use crate::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use crate::repositories::outbox::OutboxInterface;
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::repositories::pgmq::PgmqInterface;
use crate::repositories::{InvoiceInterface, SubscriptionInterface};
use crate::services::Services;
use chrono::Utc;
use common_domain::ids::TenantId;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::invoices::InvoiceRow;

impl Services {
    pub async fn on_invoice_accounting_pdf_generated(
        &self,
        event: InvoicePdfGeneratedEvent,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let invoice = self
            .store
            .get_invoice_by_id(tenant_id, event.invoice_id)
            .await?;

        let customer = self
            .store
            .find_customer_by_id(invoice.customer_id, tenant_id)
            .await?;

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(customer.invoicing_entity_id))
            .await?;

        if let Some(subscription_id) = invoice.subscription_id {
            // 3 cases here :
            // - Already paid (checkout flow). We send the initial email with receipt and invoice
            // - collection_method == SendInvoice . We send the invoice, with a payment link if applicable
            // - collection_method == ChargeAutomatically . Depending on the payment method, we trigger the payment (Invoice will be sent after receipt is generated) or fallback on payment link

            if invoice.payment_status == InvoicePaymentStatus::Paid {
                let receipt = self
                    .store
                    .last_settled_payment_tx_by_invoice_id(tenant_id, event.invoice_id)
                    .await?;

                if let Some(receipt) = receipt {
                    // TODO we send invoice paid twice without making sure we have the tx receipt. LEt's check the flow one more time
                    let label = "Thank you for your payment".to_string();

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
                        invoice_pdf_id: event.pdf_id,
                        receipt_pdf_id: receipt.receipt_pdf_id,
                    }
                    .try_into();

                    self.store
                        .pgmq_send_batch(PgmqQueue::SendEmailRequest, vec![event?])
                        .await?;

                    return Ok(());
                }
                tracing::warn!("No receipt found for invoice {}", event.invoice_id);
                return Ok(());
            }

            let subscription = self
                .store
                .get_subscription(tenant_id, subscription_id)
                .await?;

            // TODO should we save that in the invoice ? after it's paid only ?
            let subscription_payment_method = self
                .store
                .resolve_payment_method_for_subscription(tenant_id, subscription_id)
                .await?;

            match (
                subscription_payment_method,
                subscription.charge_automatically,
            ) {
                (ResolvedPaymentMethod::CustomerPaymentMethod(payment_method_id), true) => {
                    // For $0 invoices (e.g., 100% coupon), mark as paid directly without payment
                    if invoice.amount_due <= 0 {
                        self.mark_zero_amount_invoice_as_paid(tenant_id, &invoice)
                            .await?;
                        return Ok(());
                    }

                    // we trigger auto payment
                    let evt: StoreResult<PgmqMessageNew> =
                        PaymentRequestEvent::new(tenant_id, event.invoice_id, payment_method_id)
                            .try_into();

                    self.store
                        .pgmq_send_batch(PgmqQueue::PaymentRequest, vec![evt?])
                        .await?;
                }
                // In all other cases, send the invoice with payment instructions:
                // - BankTransfer: includes bank account details for wire transfer
                // - NotConfigured (External config): invoice only, no payment collection
                // - NotConfigured (no valid payment method): includes pay_online link
                // - CustomerPaymentMethod + charge_automatically=false: includes pay_online link
                _ => {
                    self.send_invoice_ready_mail(event, invoice, customer, invoicing_entity)
                        .await?;
                }
            }
        } else if invoice.manual {
            self.send_invoice_ready_mail(event, invoice, customer, invoicing_entity)
                .await?;
        }
        Ok(())
    }

    async fn send_invoice_ready_mail(
        &self,
        event: InvoicePdfGeneratedEvent,
        invoice: Invoice,
        customer: Customer,
        invoicing_entity: InvoicingEntity,
    ) -> StoreResult<()> {
        let label = invoice
            .plan_name
            .as_ref()
            .map(|plan| format!("Your {} subscription", plan))
            .unwrap_or_else(|| "Invoice for services".to_string());

        let issue_event = SendEmailRequest::InvoiceReady {
            tenant_id: invoice.tenant_id,
            invoice_id: invoice.id,
            invoicing_entity_id: invoice.seller_details.id,
            invoice_number: invoice.invoice_number,
            invoice_date: invoice.invoice_date,
            invoice_due_date: invoice.due_at.map_or(invoice.invoice_date, |d| d.date()),
            label,
            currency: invoice.currency,
            company_name: invoice.seller_details.legal_name.clone(),
            logo_attachment_id: invoicing_entity.logo_attachment_id,
            invoicing_emails: customer.invoicing_emails,
            invoice_pdf_id: event.pdf_id,
            amount_due: invoice.amount_due,
        };

        let evt: StoreResult<PgmqMessageNew> = issue_event.try_into();

        self.store
            .pgmq_send_batch(PgmqQueue::SendEmailRequest, vec![evt?])
            .await?;

        Ok(())
    }

    /// Handle $0 invoices (e.g., 100% coupon) - mark as paid and emit invoice_paid event.
    /// The `on_invoice_paid` handler will take care of subscription activation.
    async fn mark_zero_amount_invoice_as_paid(
        &self,
        tenant_id: TenantId,
        invoice: &Invoice,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                async move {
                    let now = Utc::now().naive_utc();

                    // Mark invoice as paid (no payment transaction needed for $0)
                    InvoiceRow::apply_payment_status(
                        conn,
                        invoice.id,
                        tenant_id,
                        diesel_models::enums::InvoicePaymentStatus::Paid,
                        Some(now),
                    )
                    .await?;

                    // Emit invoice paid event - this triggers on_invoice_paid which handles
                    // subscription activation (TrialExpired → Active)
                    self.store
                        .insert_outbox_event_tx(conn, OutboxEvent::invoice_paid(invoice.into()))
                        .await?;

                    tracing::info!(
                        "Marked zero-amount invoice {} as paid (e.g., 100% coupon)",
                        invoice.id
                    );

                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }
}
