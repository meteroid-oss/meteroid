use chrono::NaiveDate;
use common_repository::Client;
use cornucopia_async::Params;
use meteroid_repository as db;

use crate::{compute::InvoiceEngine, errors};
use common_utils::error_stack_conv::AnyhowIntoReport;
use error_stack::{Result, ResultExt};
use log::error;

pub async fn update_invoice_line_items(
    invoice_data: &db::invoices::Invoice,
    compute_service: &InvoiceEngine,
    db_client: &Client,
) -> Result<(), errors::WorkerError> {
    let invoice_date = convert_time_to_chrono(invoice_data.invoice_date)?;

    let invoice_lines = compute_service
        .calculate_invoice_lines(db_client, &invoice_data.subscription_id, &invoice_date)
        .await
        .map_err(|e| {
            error!("Failed to calculate invoice lines: {}", e);
            e
        })
        .into_report()
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
