use meteroid_store::domain::outbox_event::{
    CustomerEvent, InvoiceEvent, OutboxEvent, SubscriptionEvent,
};
use rdkafka::Message;
use rdkafka::message::{BorrowedHeaders, BorrowedMessage, Headers};
use serde::Deserialize;

/// This function falls back to None in case of parsing error
/// todo return Result<Option<EventType>, Error>
pub(crate) fn from_kafka_message(m: &BorrowedMessage<'_>) -> Option<OutboxEvent> {
    let headers = m.headers()?;
    let event_type = headers.get_as_string("event_type")?;

    match event_type.as_str() {
        "CustomerCreated" => {
            let payload = extract_payload::<CustomerEvent>(m).ok()??;
            Some(OutboxEvent::CustomerCreated(Box::new(payload)))
        }
        "InvoiceCreated" => {
            let payload = extract_payload::<InvoiceEvent>(m).ok()??;
            Some(OutboxEvent::InvoiceCreated(Box::new(payload)))
        }
        "InvoiceFinalized" => {
            let payload = extract_payload::<InvoiceEvent>(m).ok()??;
            Some(OutboxEvent::InvoiceFinalized(Box::new(payload)))
        }
        "SubscriptionCreated" => {
            let payload = extract_payload::<SubscriptionEvent>(m).ok()??;
            Some(OutboxEvent::SubscriptionCreated(Box::new(payload)))
        }
        _ => None,
    }
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
}

impl ParseableHeaders for &BorrowedHeaders {
    fn get_as_string(&self, key: &str) -> Option<String> {
        let header_value = self
            .iter()
            .find_map(|x| if x.key == key { x.value } else { None })?;

        String::from_utf8(header_value.to_vec()).ok()
    }
}
