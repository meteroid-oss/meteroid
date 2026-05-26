//! Webhook verification and event parsing.
//!
//! GoCardless signature scheme differs from Stripe:
//! - Header: `Webhook-Signature` (single value, hex-encoded HMAC-SHA-256 of
//!   the *raw* request body with the endpoint secret).
//! - **No** timestamp in the header — replay protection is by event-id
//!   dedup at our DB layer.
//!
//! Payload envelope: `{ "events": [ ... ] }`. A single delivery can batch
//! multiple events; the adapter normalises each one independently.

use crate::error::WebhookError;
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::collections::HashMap;

#[derive(Clone, Debug, Deserialize)]
pub struct EventEnvelope {
    pub events: Vec<Event>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Event {
    /// `EV...` prefix.
    pub id: String,
    pub created_at: String,
    /// `payments`, `mandates`, `refunds`, `subscriptions`, …
    pub resource_type: String,
    /// Resource-specific action: `confirmed`, `failed`, `paid_out`,
    /// `cancelled`, `created`, `submitted`, `customer_approval_granted`,
    /// `customer_approval_denied`, `charged_back`, etc.
    pub action: String,
    #[serde(default)]
    pub links: EventLinks,
    #[serde(default)]
    pub details: Option<EventDetails>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// Links to the affected resources. Populated subset depends on event type
/// (`payment` for payment events, `mandate` for mandate events, etc).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct EventLinks {
    pub payment: Option<String>,
    pub mandate: Option<String>,
    pub refund: Option<String>,
    pub customer: Option<String>,
    pub organisation: Option<String>,
    pub creditor: Option<String>,
    pub parent_event: Option<String>,
    pub previous_customer_bank_account: Option<String>,
    pub new_customer_bank_account: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct EventDetails {
    /// `bank` | `api` | `customer` | `gocardless` — who caused the event.
    pub origin: Option<String>,
    /// Provider-side machine code, e.g. `authorisation_disputed`,
    /// `insufficient_funds`.
    pub cause: Option<String>,
    /// Human-readable.
    pub description: Option<String>,
    /// ACH/BACS reason code, scheme-specific.
    pub scheme: Option<String>,
    pub reason_code: Option<String>,
}

pub mod resource_type {
    pub const PAYMENTS: &str = "payments";
    pub const MANDATES: &str = "mandates";
    pub const REFUNDS: &str = "refunds";
    pub const BILLING_REQUESTS: &str = "billing_requests";
}

pub mod action {
    // Payments
    pub const CREATED: &str = "created";
    pub const SUBMITTED: &str = "submitted";
    pub const CONFIRMED: &str = "confirmed";
    pub const PAID_OUT: &str = "paid_out";
    pub const FAILED: &str = "failed";
    pub const CANCELLED: &str = "cancelled";
    pub const CHARGED_BACK: &str = "charged_back";
    pub const CHARGEBACK_CANCELLED: &str = "chargeback_cancelled";
    /// Bank rejects a payment after it looked settled → funds clawed back. A
    /// FAILURE despite the name (and there is no `late_failure_resolved`).
    pub const LATE_FAILURE_SETTLED: &str = "late_failure_settled";

    // Mandates
    pub const ACTIVE: &str = "active";
    pub const EXPIRED: &str = "expired";
    pub const REPLACED: &str = "replaced";
    pub const TRANSFERRED: &str = "transferred";
    pub const CUSTOMER_APPROVAL_GRANTED: &str = "customer_approval_granted";
    pub const CUSTOMER_APPROVAL_SKIPPED: &str = "customer_approval_skipped";

    // Refunds
    pub const REFUND_SETTLED: &str = "refund_settled";
}

pub struct GoCardlessWebhook;

impl GoCardlessWebhook {
    /// Hex-encoded HMAC-SHA-256 of the raw body, **constant-time** compared.
    ///
    /// Delegates to `Mac::verify_slice` from the `hmac` crate, which uses
    /// `subtle::ConstantTimeEq` internally. Critical: a naive byte-by-byte
    /// `==` would leak the digest via timing and allow signature forgery
    /// from an attacker with enough requests.
    pub fn validate_signature(payload: &[u8], header_sig: &str, secret: &str) -> Result<(), WebhookError> {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| WebhookError::BadKey)?;
        mac.update(payload);

        let provided =
            hex::decode(header_sig.trim()).map_err(|_| WebhookError::BadSignature)?;

        mac.verify_slice(provided.as_slice())
            .map_err(|_| WebhookError::BadSignature)
    }

    pub fn parse_envelope(payload: &[u8]) -> Result<EventEnvelope, WebhookError> {
        Ok(serde_json::from_slice(payload)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "gc_whsec_test";

    fn sign(payload: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
        mac.update(payload);
        hex::encode(mac.finalize().into_bytes())
    }

    #[test]
    fn valid_signature_accepted() {
        let body = br#"{"events":[]}"#;
        let sig = sign(body);
        GoCardlessWebhook::validate_signature(body, &sig, SECRET).expect("must accept");
    }

    #[test]
    fn tampered_payload_rejected() {
        let body = br#"{"events":[]}"#;
        let sig = sign(body);
        let tampered = br#"{"events":[{}]}"#;
        assert!(GoCardlessWebhook::validate_signature(tampered, &sig, SECRET).is_err());
    }

    #[test]
    fn wrong_secret_rejected() {
        let body = br#"{"events":[]}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(b"wrong").unwrap();
        mac.update(body);
        let sig = hex::encode(mac.finalize().into_bytes());
        assert!(GoCardlessWebhook::validate_signature(body, &sig, SECRET).is_err());
    }

    #[test]
    fn parses_payment_event() {
        let body = br#"{
            "events":[{
                "id":"EV123",
                "created_at":"2026-05-19T12:00:00Z",
                "resource_type":"payments",
                "action":"confirmed",
                "links":{"payment":"PM123","organisation":"OR1"},
                "details":{"origin":"bank","cause":"payment_confirmed","description":"Payment confirmed"},
                "metadata":{"meteroid.transaction_id":"tx_xyz"}
            }]
        }"#;
        let env = GoCardlessWebhook::parse_envelope(body).unwrap();
        assert_eq!(env.events.len(), 1);
        let e = &env.events[0];
        assert_eq!(e.id, "EV123");
        assert_eq!(e.resource_type, "payments");
        assert_eq!(e.action, "confirmed");
        assert_eq!(e.links.payment.as_deref(), Some("PM123"));
        assert_eq!(
            e.metadata.get("meteroid.transaction_id").map(|s| s.as_str()),
            Some("tx_xyz")
        );
    }
}
