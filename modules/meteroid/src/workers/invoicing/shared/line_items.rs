use crate::{compute::InvoiceEngine, errors};

use error_stack::{Result, ResultExt};
use meteroid_store::domain::Invoice;

use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::Store;

pub async fn get_invoice_lines(
    invoice: &Invoice,
    compute_service: &InvoiceEngine,
    store: Store,
) -> Result<serde_json::Value, errors::WorkerError> {
    let items = match &invoice.subscription_id {
        None => invoice.line_items.clone(),
        Some(subscription_id) => {
            let subscription_details = store
                .get_subscription_details(invoice.tenant_id, subscription_id.clone())
                .await
                .change_context(errors::WorkerError::DatabaseError)?;

            let invoice_lines = compute_service
                .compute_dated_invoice_lines(&invoice.invoice_date, subscription_details)
                .await
                .change_context(errors::WorkerError::MeteringError)?;

            serde_json::to_value(invoice_lines)
                .attach_printable("Failed to encode computed invoice lines to JSON")
                .change_context(errors::WorkerError::MeteringError)?
        }
    };

    Ok(items)
}
