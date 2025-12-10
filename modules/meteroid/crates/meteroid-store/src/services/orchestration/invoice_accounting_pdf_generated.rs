use crate::StoreResult;
use crate::domain::outbox_event::InvoicePdfGeneratedEvent;
use crate::domain::pgmq::{PaymentRequestEvent, PgmqMessageNew, PgmqQueue, SendEmailRequest};
use crate::domain::{
    Customer, Invoice, InvoicePaymentStatus, InvoicingEntity, ResolvedPaymentMethod,
};
use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use crate::repositories::customers::CustomersInterfaceAuto;
use crate::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::repositories::pgmq::PgmqInterface;
use crate::repositories::{InvoiceInterface, SubscriptionInterface};
use crate::services::Services;
use common_domain::ids::TenantId;

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
                    // we trigger auto payment
                    let evt: StoreResult<PgmqMessageNew> =
                        PaymentRequestEvent::new(tenant_id, event.invoice_id, payment_method_id)
                            .try_into();

                    self.store
                        .pgmq_send_batch(PgmqQueue::PaymentRequest, vec![evt?])
                        .await?;
                }
                // in all other cases, we send the invoice with the means to pay (bank account, payment link, or "contact your account manager"))
                _ => {
                    // we send bank transfer details with invoice
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
}
