mod error;
mod outbox;
mod pdf_render;
mod processor;
pub mod processors;
mod webhook_out;

type PgmqResult<T> = error_stack::Result<T, error::PgmqError>;
