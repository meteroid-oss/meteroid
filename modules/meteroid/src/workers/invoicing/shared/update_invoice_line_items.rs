use chrono::NaiveDate;
use common_repository::Client;
use cornucopia_async::Params;
use meteroid_repository as db;

use crate::{compute2::InvoiceEngine, errors};

use error_stack::{Result, ResultExt};

use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::Store;

pub async fn update_invoice_line_items(
    invoice_data: &db::invoices::Invoice,
    compute_service: &InvoiceEngine,
    db_client: &Client,
    store: Store,
) -> Result<(), errors::WorkerError> {
    let invoice_date = convert_time_to_chrono(invoice_data.invoice_date)?;

    let subscription_details = store
        .get_subscription_details(
            invoice_data.tenant_id.clone(),
            invoice_data.subscription_id.clone(),
        )
        .await
        .change_context(errors::WorkerError::DatabaseError)?;

    let invoice_lines = compute_service
        .compute_dated_invoice_lines(&invoice_date, subscription_details)
        .await
        .change_context(errors::WorkerError::MeteringError)?;

    let invoice_lines_json = serde_json::to_value(invoice_lines)
        .attach_printable("Failed to encode computed invoice lines to JSON")
        .change_context(errors::WorkerError::MeteringError)?; // TODO

    let params = db::invoices::UpdateInvoiceLinesParams {
        id: invoice_data.id,
        line_items: invoice_lines_json,
    };

    db::invoices::update_invoice_lines()
        .params(db_client, &params)
        .await
        .change_context(errors::WorkerError::DatabaseError)?;

    Ok(())
}

fn convert_time_to_chrono(time_date: time::Date) -> Result<NaiveDate, errors::WorkerError> {
    NaiveDate::from_ymd_opt(
        time_date.year(),
        time_date.month() as u32,
        time_date.day() as u32,
    )
    .ok_or(errors::WorkerError::InvalidInput.into())
}
