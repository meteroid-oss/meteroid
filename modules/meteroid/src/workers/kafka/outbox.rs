use chrono::{DateTime, Utc};
use meteroid_store::domain::outbox_event::{CustomerEvent, InvoiceEvent, SubscriptionEvent};
use rdkafka::message::{BorrowedHeaders, BorrowedMessage, Headers};
use rdkafka::Message;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug)]
pub struct OutboxEvent {
    pub id: String,
    pub tenant_id: Uuid,
    pub aggregate_id: String,
    pub event_type: EventType,
    pub event_timestamp: DateTime<Utc>,
}

#[derive(Debug)]
pub enum EventType {
    CustomerCreated(Box<CustomerEvent>),
    InvoiceCreated(Box<InvoiceEvent>),
    InvoiceFinalized(Box<InvoiceEvent>),
    InvoicePdfRequested,
    SubscriptionCreated(Box<SubscriptionEvent>),
}

impl EventType {
    /// This function falls back to None in case of parsing error
    /// todo return Result<Option<EventType>, Error>
    pub fn from_kafka_message(m: &BorrowedMessage<'_>) -> Option<Self> {
        let headers = m.headers()?;
        let event_type = headers.get_as_string("event_type")?;

        match event_type.as_str() {
            "customer.created" => {
                let payload = extract_payload::<CustomerEvent>(m).ok()??;
                Some(Self::CustomerCreated(Box::new(payload)))
            }
            "invoice.created" => {
                let payload = extract_payload::<InvoiceEvent>(m).ok()??;
                Some(Self::InvoiceCreated(Box::new(payload)))
            }
            "invoice.finalized" => {
                let payload = extract_payload::<InvoiceEvent>(m).ok()??;
                Some(Self::InvoiceFinalized(Box::new(payload)))
            }
            "invoice.pdf.requested" => Some(Self::InvoicePdfRequested),
            "subscription.created" => {
                let payload = extract_payload::<SubscriptionEvent>(m).ok()??;
                Some(Self::SubscriptionCreated(Box::new(payload)))
            }
            _ => None,
        }
    }
}

/// This function falls back to None in case of parsing error
/// todo return Result<Option<OutboxEvent>, Error>
pub(crate) fn parse_outbox_event(m: &BorrowedMessage<'_>) -> Option<OutboxEvent> {
    let headers = m.headers()?;
    let id = headers.get_as_string("local_id")?;
    let tenant_id = headers.get_as_uuid("tenant_id")?;

    let aggregate_id: String = String::from_utf8(m.key()?.to_vec()).ok()?;

    let event_type = EventType::from_kafka_message(m)?;

    let event_timestamp = DateTime::from_timestamp_millis(m.timestamp().to_millis()?)?;

    Some(OutboxEvent {
        id,
        tenant_id,
        aggregate_id,
        event_type,
        event_timestamp,
    })
}

fn extract_payload<P: for<'a> Deserialize<'a>>(
    m: &BorrowedMessage<'_>,
) -> Result<Option<P>, serde_json::Error> {
    if let Some(payload) = m.payload() {
        let parsed = serde_json::from_slice(payload)?;
        Ok(Some(parsed))
    } else {
        Ok(None)
    }
}

trait ParseableHeaders {
    fn get_as_string(&self, key: &str) -> Option<String>;
    fn get_as_uuid(&self, key: &str) -> Option<Uuid>;
}

impl ParseableHeaders for &BorrowedHeaders {
    fn get_as_string(&self, key: &str) -> Option<String> {
        let header_value = self
            .iter()
            .find_map(|x| if x.key == key { x.value } else { None })?;

        String::from_utf8(header_value.to_vec()).ok()
    }

    fn get_as_uuid(&self, key: &str) -> Option<Uuid> {
        self.get_as_string(key)
            .and_then(|header_value| Uuid::parse_str(&header_value).ok())
    }
}
