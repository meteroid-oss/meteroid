use error_stack::Report;

mod billable_metric_sync;
mod error;
mod hubspot_sync;

mod outbox;
mod pdf_render;
mod pennylane_sync;
mod processor;
pub mod processors;
mod send_email;
mod webhook_out;

mod invoice_orchestration;

type PgmqResult<T> = Result<T, Report<error::PgmqError>>;
