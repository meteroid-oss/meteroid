use error_stack::Report;

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
