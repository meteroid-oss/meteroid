use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use common_domain::ids::InvoiceId;
use common_domain::pgmq::{Headers, Message, MessageId, ReadCt};
use diesel_models::pgmq::{PgmqMessageRow, PgmqMessageRowNew};
use o2o::o2o;
use serde::{Deserialize, Serialize};
use strum::Display;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum PgmqQueue {
    OutboxEvent,
    InvoicePdfRequest,
    WebhookOut,
}

impl PgmqQueue {
    pub fn as_str(&self) -> &'static str {
        match self {
            PgmqQueue::OutboxEvent => "outbox_event",
            PgmqQueue::InvoicePdfRequest => "invoice_pdf_request",
            PgmqQueue::WebhookOut => "webhook_out",
        }
    }
}

#[derive(Debug, Clone, o2o)]
#[from_owned(PgmqMessageRow)]
pub struct PgmqMessage {
    pub msg_id: MessageId,
    pub message: Message,
    pub headers: Headers,
    pub read_ct: ReadCt,
}

#[derive(Debug, Clone, o2o)]
#[owned_into(PgmqMessageRowNew)]
pub struct PgmqMessageNew {
    pub message: Message,
    pub headers: Headers,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoicePdfRequestEvent {
    pub invoice_id: InvoiceId,
}

impl InvoicePdfRequestEvent {
    pub fn new(invoice_id: InvoiceId) -> Self {
        Self { invoice_id }
    }
}

json_value_serde!(InvoicePdfRequestEvent);

impl TryInto<PgmqMessageNew> for InvoicePdfRequestEvent {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<PgmqMessageNew, Self::Error> {
        let headers = Headers::none();
        let message = Message::some(self.try_into()?);

        Ok(PgmqMessageNew { message, headers })
    }
}

impl TryInto<InvoicePdfRequestEvent> for &PgmqMessage {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<InvoicePdfRequestEvent, Self::Error> {
        let payload = self
            .message
            .0
            .as_ref()
            .ok_or(StoreError::ValueNotFound("Pgmq message".to_string()))?;

        payload.try_into()
    }
}
