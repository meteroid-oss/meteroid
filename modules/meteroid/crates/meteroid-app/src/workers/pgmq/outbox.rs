//! Outbox-event decoding + per-consumer transforms.
//!
//! Producers publish each event to its routing key (`OutboxEvent::routing_key`),
//! and pgmq fans a copy into every bound consumer queue. Consumers decode the raw
//! outbox event off their queue and apply the transform below.

use crate::workers::pgmq::processor::HandleResult;
use common_domain::pgmq::MessageId;
use meteroid_store::StoreResult;
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::{
    HubspotSyncRequestEvent, PennylaneSyncCreditNote, PennylaneSyncInvoice,
    PennylaneSyncRequestEvent, PgmqMessage, QuoteConversionRequestEvent, VatValidationRequestEvent,
};

/// Per-message decode outcome: an undecodable or unroutable message fails on its own
/// instead of failing (and stalling) the whole batch.
pub struct Decoded<T> {
    pub events: Vec<(MessageId, T)>,
    pub failed: Vec<(MessageId, String)>,
}

/// `decode` returns `Ok(None)` for a message this consumer has no route for.
pub fn decode_each<T>(
    msgs: &[PgmqMessage],
    decode: impl Fn(&PgmqMessage) -> StoreResult<Option<T>>,
) -> Decoded<T> {
    let mut decoded = Decoded {
        events: Vec::with_capacity(msgs.len()),
        failed: vec![],
    };
    for msg in msgs {
        match decode(msg) {
            Ok(Some(evt)) => decoded.events.push((msg.msg_id, evt)),
            Ok(None) => decoded
                .failed
                .push((msg.msg_id, "Unroutable message for this queue".to_string())),
            Err(err) => decoded.failed.push(HandleResult::fail(msg.msg_id, &err)),
        }
    }
    decoded
}

pub fn to_outbox_events(msgs: &[PgmqMessage]) -> Decoded<OutboxEvent> {
    decode_each(msgs, |msg| {
        let event: StoreResult<OutboxEvent> = msg.try_into();
        event.map(Some)
    })
}

/// Decodes a native `T` from a direct enqueue, else transforms a topic-routed outbox event.
pub fn native_or_outbox<T>(
    msgs: &[PgmqMessage],
    from_outbox: impl Fn(OutboxEvent) -> Option<T>,
) -> Decoded<T>
where
    for<'a> &'a PgmqMessage: TryInto<T>,
{
    decode_each(msgs, |msg| {
        if let Ok(evt) = TryInto::<T>::try_into(msg) {
            return Ok(Some(evt));
        }
        let event: StoreResult<OutboxEvent> = TryInto::<OutboxEvent>::try_into(msg);
        event.map(&from_outbox)
    })
}

// Raw-outbox-event -> consumer-native-event transforms, applied when a topic-routed
// event lands on a consumer queue.

/// `None` for events the hubspot queue isn't bound to (defensive).
pub fn hubspot_from_outbox(event: OutboxEvent) -> Option<HubspotSyncRequestEvent> {
    match event {
        OutboxEvent::CustomerCreated(e) | OutboxEvent::CustomerUpdated(e) => {
            Some(HubspotSyncRequestEvent::CustomerOutbox(e))
        }
        OutboxEvent::SubscriptionCreated(e) => Some(HubspotSyncRequestEvent::SubscriptionOutbox(e)),
        _ => None,
    }
}

pub fn pennylane_from_outbox(event: OutboxEvent) -> Option<PennylaneSyncRequestEvent> {
    match event {
        OutboxEvent::CustomerCreated(e) | OutboxEvent::CustomerUpdated(e) => {
            Some(PennylaneSyncRequestEvent::CustomerOutbox(e))
        }
        OutboxEvent::InvoiceAccountingPdfGenerated(e) => Some(PennylaneSyncRequestEvent::Invoice(
            Box::new(PennylaneSyncInvoice {
                id: e.invoice_id,
                tenant_id: e.tenant_id,
            }),
        )),
        OutboxEvent::InvoicePaid(e) => Some(PennylaneSyncRequestEvent::Invoice(Box::new(
            PennylaneSyncInvoice {
                id: e.invoice_id,
                tenant_id: e.tenant_id,
            },
        ))),
        OutboxEvent::CreditNoteFinalized(e) => Some(PennylaneSyncRequestEvent::CreditNote(
            Box::new(PennylaneSyncCreditNote {
                id: e.credit_note_id,
                tenant_id: e.tenant_id,
            }),
        )),
        _ => None,
    }
}

/// `None` when the VAT number is absent, malformed, or VIES-ineligible.
pub fn vat_from_outbox(event: OutboxEvent) -> Option<VatValidationRequestEvent> {
    let evt = match event {
        OutboxEvent::CustomerCreated(e) | OutboxEvent::CustomerUpdated(e) => e,
        _ => return None,
    };

    let vat_number = evt.vat_number.as_ref()?;
    if !meteroid_tax::validation::validate_vat_number_format(vat_number)
        || !meteroid_tax::vies::is_vies_eligible(vat_number)
    {
        return None;
    }

    Some(VatValidationRequestEvent {
        tenant_id: evt.tenant_id,
        customer_id: evt.customer_id,
        vat_number: vat_number.clone(),
        attempt: 0,
        revalidate: false,
    })
}

pub fn quote_from_outbox(event: OutboxEvent) -> Option<QuoteConversionRequestEvent> {
    match event {
        OutboxEvent::QuoteAccepted(e) => Some(QuoteConversionRequestEvent::QuoteAccepted(e)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::pgmq::{Message, ReadCt};
    use error_stack::Report;
    use meteroid_store::errors::StoreError;

    fn msg(id: i64, kind: &str) -> PgmqMessage {
        PgmqMessage {
            msg_id: MessageId(id),
            message: Some(Message(serde_json::json!({ "kind": kind }))),
            headers: None,
            read_ct: ReadCt(1),
            enqueued_at: common_domain::clock::now().naive_utc(),
        }
    }

    #[test]
    fn decode_each_isolates_undecodable_and_unroutable_messages() {
        let msgs = [
            msg(1, "ok"),
            msg(2, "garbage"),
            msg(3, "other"),
            msg(4, "ok"),
        ];

        let decoded = decode_each(&msgs, |m| {
            match m.message.as_ref().unwrap().0["kind"].as_str() {
                Some("ok") => Ok(Some(m.msg_id.0)),
                Some("other") => Ok(None),
                _ => Err(Report::new(StoreError::ValueNotFound("kind".to_string()))),
            }
        });

        let ok: Vec<_> = decoded.events.iter().map(|(id, _)| id.0).collect();
        let failed: Vec<_> = decoded.failed.iter().map(|(id, _)| id.0).collect();
        assert_eq!(ok, vec![1, 4]);
        assert_eq!(failed, vec![2, 3]);
    }
}
