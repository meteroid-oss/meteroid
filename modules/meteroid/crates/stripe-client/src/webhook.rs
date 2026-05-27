use crate::error::WebhookError;
use crate::payment_intents::StripePaymentIntent;
use crate::payment_methods::PaymentMethod;
use crate::setup_intents::SetupIntent;
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::collections::HashMap;

pub mod event_type {
    // Payment intents
    pub const PAYMENT_INTENT_SUCCEEDED: &str = "payment_intent.succeeded";
    pub const PAYMENT_INTENT_FAILED: &str = "payment_intent.payment_failed";
    pub const PAYMENT_INTENT_REQUIRES_ACTION: &str = "payment_intent.requires_action";
    pub const PAYMENT_INTENT_PROCESSING: &str = "payment_intent.processing";
    pub const PAYMENT_INTENT_PARTIALLY_FUNDED: &str = "payment_intent.partially_funded";

    // Setup intents
    pub const SETUP_INTENT_SUCCEEDED: &str = "setup_intent.succeeded";
    pub const SETUP_INTENT_REQUIRES_ACTION: &str = "setup_intent.requires_action";
    pub const SETUP_INTENT_CANCELED: &str = "setup_intent.canceled";

    // Charges (refunds piggy-back on the parent charge object)
    pub const CHARGE_REFUNDED: &str = "charge.refunded";

    // Disputes
    pub const CHARGE_DISPUTE_CREATED: &str = "charge.dispute.created";
    pub const CHARGE_DISPUTE_CLOSED: &str = "charge.dispute.closed";
    pub const CHARGE_DISPUTE_FUNDS_WITHDRAWN: &str = "charge.dispute.funds_withdrawn";
    pub const CHARGE_DISPUTE_FUNDS_REINSTATED: &str = "charge.dispute.funds_reinstated";

    // Payment-method lifecycle (card-expiring async flow)
    pub const PAYMENT_METHOD_UPDATED: &str = "payment_method.updated";
    pub const PAYMENT_METHOD_DETACHED: &str = "payment_method.detached";
    pub const PAYMENT_METHOD_AUTO_UPDATED: &str = "payment_method.automatically_updated";

    // Mandates (SEPA / BACS / ACH / SCA)
    pub const MANDATE_UPDATED: &str = "mandate.updated";
}

/// Events we self-register the webhook endpoint for; also the canonical list of
/// what `normalize_event` knows how to handle.
pub static STRIPE_PAYMENT_WEBHOOKS: &[&str] = &[
    event_type::PAYMENT_INTENT_SUCCEEDED,
    event_type::PAYMENT_INTENT_FAILED,
    event_type::PAYMENT_INTENT_REQUIRES_ACTION,
    event_type::PAYMENT_INTENT_PROCESSING,
    event_type::PAYMENT_INTENT_PARTIALLY_FUNDED,
    event_type::SETUP_INTENT_SUCCEEDED,
    event_type::SETUP_INTENT_REQUIRES_ACTION,
    event_type::SETUP_INTENT_CANCELED,
    event_type::CHARGE_REFUNDED,
    event_type::CHARGE_DISPUTE_CREATED,
    event_type::CHARGE_DISPUTE_CLOSED,
    event_type::CHARGE_DISPUTE_FUNDS_WITHDRAWN,
    event_type::CHARGE_DISPUTE_FUNDS_REINSTATED,
    event_type::PAYMENT_METHOD_UPDATED,
    event_type::PAYMENT_METHOD_DETACHED,
    event_type::PAYMENT_METHOD_AUTO_UPDATED,
    event_type::MANDATE_UPDATED,
];

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "object", rename_all = "snake_case")]
pub enum EventObject {
    PaymentIntent(StripePaymentIntent),
    SetupIntent(SetupIntent),
    PaymentMethod(PaymentMethod),
    Charge(StripeCharge),
    Dispute(StripeDispute),
    Mandate(StripeMandate),
}

/// Charge object, narrowed to the fields we use for `charge.refunded` events.
#[derive(Clone, Debug, Deserialize)]
pub struct StripeCharge {
    pub id: String,
    /// Parent PaymentIntent id; present for PI-based charges, absent on legacy Charges API flows.
    pub payment_intent: Option<String>,
    pub amount: i64,
    pub amount_refunded: i64,
    pub currency: String,
    /// Inlined in `charge.refunded` payloads even though it's not expanded by default on retrieve.
    pub refunds: Option<StripeRefundList>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StripeRefundList {
    pub data: Vec<StripeRefund>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StripeRefund {
    pub id: String,
    pub amount: i64,
    pub currency: Option<String>,
    pub status: Option<String>,
    pub payment_intent: Option<String>,
    pub charge: Option<String>,
}

/// Same shape for all four dispute event types; `Event::event_type` discriminates.
#[derive(Clone, Debug, Deserialize)]
pub struct StripeDispute {
    pub id: String,
    pub charge: String,
    pub payment_intent: Option<String>,
    pub amount: i64,
    pub currency: String,
    pub reason: String,
    pub status: String,
}

/// Mandate object — surfaced when SEPA/BACS/ACH or SCA mandates change status.
#[derive(Clone, Debug, Deserialize)]
pub struct StripeMandate {
    pub id: String,
    pub status: StripeMandateStatus,
    pub payment_method: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StripeMandateStatus {
    Active,
    Inactive,
    Pending,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NotificationEventData {
    pub object: EventObject,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Event {
    pub id: String,

    pub data: NotificationEventData,

    #[serde(rename = "type")]
    pub event_type: String,

    /// Unix seconds the event occurred at Stripe; use this (not server-side now)
    /// downstream — matters for dispute-window math and dashboard replays.
    #[serde(default)]
    pub created: Option<i64>,
}

pub struct StripeWebhook {
    current_timestamp: i64,
}

impl StripeWebhook {
    pub fn validate_signature(payload: &str, sig: &str, secret: &str) -> Result<(), WebhookError> {
        Self {
            current_timestamp: Utc::now().timestamp(),
        }
        .do_validate_signature(payload, sig, secret)
    }

    pub fn parse_event(payload: &str) -> Result<Event, WebhookError> {
        Ok(serde_json::from_str(payload)?)
    }

    fn do_validate_signature(
        self,
        payload: &str,
        sig: &str,
        secret: &str,
    ) -> Result<(), WebhookError> {
        let signature = Signature::parse(sig)?;
        let signed_payload = format!("{}.{}", signature.t, payload);

        let mut mac =
            Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| WebhookError::BadKey)?;
        mac.update(signed_payload.as_bytes());

        let sig = hex::decode(signature.v1).map_err(|_| WebhookError::BadSignature)?;

        mac.verify_slice(sig.as_slice())
            .map_err(|_| WebhookError::BadSignature)?;

        // Reject signatures outside a 5-minute tolerance to limit replay.
        if (self.current_timestamp - signature.t).abs() > 300 {
            return Err(WebhookError::BadTimestamp(signature.t));
        }

        Ok(())
    }
}

struct Signature<'r> {
    t: i64,
    v1: &'r str,
}

impl<'r> Signature<'r> {
    fn parse(raw: &'r str) -> Result<Signature<'r>, WebhookError> {
        let headers: HashMap<&str, &str> = raw
            .split(',')
            .map(|header| {
                let mut key_and_value = header.split('=');
                let key = key_and_value.next();
                let value = key_and_value.next();
                (key, value)
            })
            .filter_map(|(key, value)| match (key, value) {
                (Some(key), Some(value)) => Some((key, value)),
                _ => None,
            })
            .collect();
        let t = headers.get("t").ok_or(WebhookError::BadSignature)?;
        let v1 = headers.get("v1").ok_or(WebhookError::BadSignature)?;
        Ok(Signature {
            t: t.parse::<i64>().map_err(WebhookError::BadHeader)?,
            v1,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_signature_parse() {
        use super::Signature;

        let raw_signature =
            "t=1492774577,v1=5257a869e7ecebeda32affa62cdca3fa51cad7e77a0e56ff536d0ce8e108d8bd";
        let signature = Signature::parse(raw_signature).unwrap();
        assert_eq!(signature.t, 1492774577);
        assert_eq!(
            signature.v1,
            "5257a869e7ecebeda32affa62cdca3fa51cad7e77a0e56ff536d0ce8e108d8bd"
        );

        let raw_signature_with_test_mode = "t=1492774577,v1=5257a869e7ecebeda32affa62cdca3fa51cad7e77a0e56ff536d0ce8e108d8bd,v0=6ffbb59b2300aae63f272406069a9788598b792a944a07aba816edb039989a39";
        let signature = Signature::parse(raw_signature_with_test_mode).unwrap();
        assert_eq!(signature.t, 1492774577);
        assert_eq!(
            signature.v1,
            "5257a869e7ecebeda32affa62cdca3fa51cad7e77a0e56ff536d0ce8e108d8bd"
        );
    }
}
