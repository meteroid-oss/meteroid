mod error;
mod hubspot_sync;
mod outbox;
mod pdf_render;
mod pennylane_sync;
mod processor;
pub mod processors;
mod webhook_out;

type PgmqResult<T> = error_stack::Result<T, error::PgmqError>;
