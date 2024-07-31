use crate::errors;

use error_stack::{Result, ResultExt};
use meteroid_store::compute::InvoiceLineInterface;
use meteroid_store::domain::{Invoice, LineItem};

use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::Store;

pub async fn get_invoice_lines(
    invoice: &Invoice,
    store: Store,
) -> Result<Vec<LineItem>, errors::WorkerError> {
    let lines = match &invoice.subscription_id {
        None => invoice.line_items.clone(),
        Some(subscription_id) => {
            let subscription_details = store
                .get_subscription_details(invoice.tenant_id, subscription_id.clone())
                .await
                .change_context(errors::WorkerError::DatabaseError)?;

            let invoice_lines = store
                .compute_dated_invoice_lines(&invoice.invoice_date, subscription_details)
                .await
                .change_context(errors::WorkerError::MeteringError)?;

            invoice_lines
        }
    };
    Ok(lines)
}
