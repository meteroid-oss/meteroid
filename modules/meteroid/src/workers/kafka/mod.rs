pub mod avro;
mod outbox;
mod pdf_renderer;
mod processor;
pub mod processors;
mod webhook;

pub const CUSTOMER_OUTBOX_TOPIC: &str = "outbox.event.customer";
pub const INVOICE_OUTBOX_TOPIC: &str = "outbox.event.invoice";
