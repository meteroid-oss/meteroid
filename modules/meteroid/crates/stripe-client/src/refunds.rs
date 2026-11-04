//! Refunds against a PaymentIntent (`POST /v1/refunds`, `GET /v1/refunds/{id}`).
//!
//! The [`StripeRefund`] object is shared with [`crate::webhook`]: `charge.refunded`
//! embeds a list of it, and `refund.updated` / `charge.refund.updated` deliver one.

use crate::client::StripeClient;
use crate::error::StripeError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize)]
pub struct CreateRefund {
    pub payment_intent: String,
    /// `None` refunds the charge's whole remaining amount.
    pub amount: Option<i64>,
    pub reason: Option<StripeRefundReason>,
    pub metadata: HashMap<String, String>,
}

/// Stripe accepts only these three; free-text reasons belong in `metadata`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StripeRefundReason {
    Duplicate,
    Fraudulent,
    RequestedByCustomer,
}

/// `status` and `currency` stay untyped strings so a status Stripe adds later
/// can't dead-letter a webhook; callers map the ones they know.
#[derive(Clone, Debug, Deserialize)]
pub struct StripeRefund {
    pub id: String,
    pub amount: i64,
    /// Unix seconds the refund was created at Stripe.
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub payment_intent: Option<String>,
    #[serde(default)]
    pub charge: Option<String>,
    /// Populated on `status = "failed"` (`lost_or_stolen_card`, `expired_or_canceled_card`,
    /// `charge_for_pending_refund_disputed`, `insufficient_funds`, `declined`, `unknown`).
    #[serde(default)]
    pub failure_reason: Option<String>,
    /// Echoes what we stamped on create (`meteroid.tenant_id`, `meteroid.refund_id`, …).
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

pub mod refund_status {
    pub const PENDING: &str = "pending";
    pub const SUCCEEDED: &str = "succeeded";
    pub const FAILED: &str = "failed";
    pub const CANCELED: &str = "canceled";
    pub const REQUIRES_ACTION: &str = "requires_action";
}

#[async_trait::async_trait]
pub trait RefundApi {
    async fn create_refund(
        &self,
        params: CreateRefund,
        secret_key: &SecretString,
        idempotency_key: String,
    ) -> Result<StripeRefund, StripeError>;

    /// Read path for the reconcile sweep: resolves a refund row left `Pending`
    /// because the outbound call timed out or the webhook never arrived.
    async fn get_refund(
        &self,
        id: &str,
        secret_key: &SecretString,
    ) -> Result<StripeRefund, StripeError>;
}

#[async_trait::async_trait]
impl RefundApi for StripeClient {
    async fn create_refund(
        &self,
        params: CreateRefund,
        secret_key: &SecretString,
        idempotency_key: String,
    ) -> Result<StripeRefund, StripeError> {
        self.post_form(
            "/refunds",
            params,
            secret_key,
            idempotency_key,
            RetryStrategy::default(),
        )
        .await
    }

    async fn get_refund(
        &self,
        id: &str,
        secret_key: &SecretString,
    ) -> Result<StripeRefund, StripeError> {
        self.get(
            &format!("/refunds/{id}"),
            secret_key,
            RetryStrategy::default(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webhook::{EventObject, StripeWebhook};

    /// `refund.updated` carries a bare refund object; a failed one must surface
    /// its `failure_reason` rather than dead-letter.
    #[test]
    fn refund_updated_event_parses() {
        let body = r#"{
  "id": "evt_refund_updated_1",
  "object": "event",
  "created": 1721728800,
  "type": "refund.updated",
  "data": {
    "object": {
      "id": "re_test_9",
      "object": "refund",
      "amount": 1500,
      "charge": "ch_test_9",
      "currency": "eur",
      "failure_reason": "expired_or_canceled_card",
      "payment_intent": "pi_test_9",
      "reason": "requested_by_customer",
      "status": "failed"
    }
  }
}"#;

        let event = StripeWebhook::parse_event(body).expect("parse refund.updated");
        assert_eq!(event.event_type, "refund.updated");

        let refund = match event.data.object {
            EventObject::Refund(refund) => refund,
            other => panic!("expected a Refund event object, got {other:?}"),
        };
        assert_eq!(refund.id, "re_test_9");
        assert_eq!(refund.amount, 1500);
        assert_eq!(refund.status.as_deref(), Some(refund_status::FAILED));
        assert_eq!(
            refund.failure_reason.as_deref(),
            Some("expired_or_canceled_card")
        );
        assert_eq!(refund.payment_intent.as_deref(), Some("pi_test_9"));
    }

    /// A refund that only names its charge (legacy Charges API flow) still parses.
    #[test]
    fn refund_without_payment_intent_parses() {
        let refund: StripeRefund = serde_json::from_str(
            r#"{"id":"re_x","object":"refund","amount":100,"charge":"ch_x","status":"succeeded"}"#,
        )
        .expect("parse refund");
        assert!(refund.payment_intent.is_none());
        assert_eq!(refund.charge.as_deref(), Some("ch_x"));
        assert!(refund.currency.is_none());
    }
}
