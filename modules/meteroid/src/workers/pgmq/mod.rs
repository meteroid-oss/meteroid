use error_stack::Report;
use rand::RngExt;
use std::time::Duration;

mod bi_aggregation;
mod billable_metric_sync;
mod credit_note_pdf_render;
mod error;
mod hubspot_sync;

mod invoice_orchestration;
mod outbox;
mod payment_request;
mod pdf_render;
mod pennylane_sync;
mod processor;
pub mod processors;
mod quote_conversion;
mod send_email;
mod webhook_out;

type PgmqResult<T> = Result<T, Report<error::PgmqError>>;

fn jitter_for_duration(base: Duration) -> u64 {
    // ~10-20% jitter, with a floor and ceiling
    (base.as_millis() as u64 / 10).clamp(10, 100)
}

pub async fn sleep_with_jitter(duration: Duration) {
    let jitter = rand::rng().random_range(0..=jitter_for_duration(duration));
    let total_duration = duration + Duration::from_millis(jitter);

    tokio::time::sleep(total_duration).await;
}
