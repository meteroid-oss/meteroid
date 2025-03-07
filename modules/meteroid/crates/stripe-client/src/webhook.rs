use crate::error::WebhookError;
use crate::payment_intents::PaymentIntent;
use crate::setup_intents::SetupIntent;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::collections::HashMap;

pub mod event_type {
    pub const SETUP_INTENT_SUCCEEDED: &str = "setup_intent.succeeded";
    pub const PAYMENT_INTENT_SUCCEEDED: &str = "payment_intent.succeeded";
    pub const PAYMENT_INTENT_FAILED: &str = "payment_intent.payment_failed";
    pub const PAYMENT_INTENT_PARTIALLY_FUNDED: &str = "payment_intent.partially_funded";
}

pub static STRIPE_PAYMENT_WEBHOOKS: [&str; 4] = [
    event_type::SETUP_INTENT_SUCCEEDED,
    event_type::PAYMENT_INTENT_SUCCEEDED,
    event_type::PAYMENT_INTENT_FAILED,
    event_type::PAYMENT_INTENT_PARTIALLY_FUNDED,
];

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "object", rename_all = "snake_case")]
pub enum EventObject {
    PaymentIntent(PaymentIntent),
    SetupIntent(SetupIntent),
}

#[derive(Clone, Debug, Deserialize)]
pub struct NotificationEventData {
    pub object: EventObject,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Event {
    /// Unique identifier for the object.
    pub id: String,

    pub data: NotificationEventData,

    #[serde(rename = "type")]
    pub event_type: String,
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
        // Get Stripe signature from header
        let signature = Signature::parse(sig)?;
        let signed_payload = format!("{}.{}", signature.t, payload);

        // Compute HMAC with the SHA256 hash function, using endpoint secret as key
        // and signed_payload string as the message.
        let mut mac =
            Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| WebhookError::BadKey)?;
        mac.update(signed_payload.as_bytes());

        let sig = hex::decode(signature.v1).map_err(|_| WebhookError::BadSignature)?;

        mac.verify_slice(sig.as_slice())
            .map_err(|_| WebhookError::BadSignature)?;

        // Get current timestamp to compare to signature timestamp
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
