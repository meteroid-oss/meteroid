use common_domain::ids::TenantId;
use crate::domain::pgmq::{PgmqMessageNew, PgmqQueue, SendEmailRequest};
use crate::{ StoreResult};
use crate::domain::outbox_event::InvoiceEvent;
use crate::repositories::{CustomersInterface, InvoiceInterface};
use crate::repositories::invoicing_entities::InvoicingEntityInterface;
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::repositories::pgmq::PgmqInterface;
use crate::services::Services;

impl Services {

pub async fn on_invoice_paid(
    &self,
    event: InvoiceEvent,
    tenant_id: TenantId
) -> StoreResult<()> {
    let receipt = self.store
        .last_settled_payment_tx_by_invoice_id(tenant_id, event.invoice_id)
        .await ?
        ;

    let invoice = self.store.get_invoice_by_id(
        tenant_id,
        event.invoice_id
    ).await ?;

    if invoice.issued_at.is_some() {
        return Ok(())
    }

    let invoice_pdf_id = match invoice.pdf_document_id {
        Some(id) => id,
        None => {
            tracing::warn!("Invoice {} has no pdf document id", invoice.id);
            return Ok(())
        }
    };

    let receipt = match receipt {
        Some(receipt) => receipt,
        None => {
            tracing::warn!("No receipt found for invoice {}", event.invoice_id);
            return Ok(())
        }
    };

    let customer = self.store.find_customer_by_id(
        invoice.customer_id,
        tenant_id,
    ).await ?;

    let invoicing_entity = self.store
        .get_invoicing_entity(
            tenant_id,
            Some(customer.invoicing_entity_id)
        )
        .await ?;

    let event : StoreResult<PgmqMessageNew> = SendEmailRequest::InvoicePaid {
        invoice_id: invoice.id,
        invoice_number: invoice.invoice_number,
        invoicing_entity_id: invoicing_entity.id,
        invoice_date: invoice.invoice_date,
        invoice_due_date: invoice.due_at.map_or(invoice.invoice_date, |d| d.date()),
        label: invoice.plan_name.unwrap_or(invoice.customer_details.name.clone()),
        amount_paid: receipt.amount,
        currency: invoice.currency,
        company_name: invoice.customer_details.name.clone(),
        logo_attachment_id: invoicing_entity.logo_attachment_id,
        invoicing_emails: customer.invoicing_emails,
        invoice_pdf_id: invoice_pdf_id,
        receipt_pdf_id: receipt.receipt_pdf_id,
    }.try_into();

    self.store.pgmq_send_batch(
        PgmqQueue::SendEmailRequest,
        vec![event?]
    ).await?;

    Ok(())
}

}
