use crate::client::StripeClient;
use crate::error::StripeError;
use crate::request::RetryStrategy;
use crate::setup_intents::StripeMandateRequest;
use crate::setup_intents::StripePaymentMethodType;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;

#[skip_serializing_none]
#[derive(Debug, Eq, PartialEq, Serialize)]
pub struct PaymentIntentRequest {
    pub amount: i64,
    pub currency: String,
    pub metadata: HashMap<String, String>,
    pub return_url: Option<String>,
    pub confirm: bool,
    pub payment_method: String,
    pub customer: Option<String>,
    #[serde(flatten)]
    pub setup_mandate_details: Option<StripeMandateRequest>,
    pub capture_method: StripeCaptureMethod,
    pub off_session: Option<bool>,
    pub payment_method_types: Vec<StripePaymentMethodType>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StripePaymentIntent {
    pub id: String,
    pub amount: i64,
    pub amount_received: Option<i64>,
    pub currency: String,
    /// Populated when the customer must complete an extra step (3DS, microdeposit,
    /// bank-app SCA); off-session charges hitting SCA return status `requires_action`.
    pub next_action: Option<StripeNextAction>,
    pub livemode: bool,
    /// Returned on PaymentIntent creation. The on-session portal needs it to
    /// complete a `requires_action` charge via Stripe.js `handleNextAction`.
    pub client_secret: Option<String>,
    pub status: StripePaymentStatus,
    pub last_payment_error: Option<StripePaymentError>,
    pub metadata: HashMap<String, String>,
}

/// Stripe's `last_payment_error` is a rich object (never a bare string per the
/// API), but we accept a string too so an unexpected shape can't dead-letter the
/// webhook. We only surface the fields we use; unknown ones — including the nested
/// `payment_method`, itself a string or an expanded object — are ignored.
#[derive(Clone, Debug, Default)]
pub struct StripePaymentError {
    pub code: Option<String>,
    pub decline_code: Option<String>,
    pub message: Option<String>,
    pub error_type: Option<String>,
}

impl StripePaymentError {
    /// Best-effort human-readable description for storage and display.
    pub fn to_message(&self) -> String {
        self.message
            .clone()
            .or_else(|| self.decline_code.clone())
            .or_else(|| self.code.clone())
            .or_else(|| self.error_type.clone())
            .unwrap_or_else(|| "Payment failed".to_string())
    }
}

impl<'de> Deserialize<'de> for StripePaymentError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Text(String),
            Object {
                code: Option<String>,
                decline_code: Option<String>,
                message: Option<String>,
                #[serde(rename = "type")]
                error_type: Option<String>,
            },
        }

        Ok(match Raw::deserialize(deserializer)? {
            Raw::Text(message) => StripePaymentError {
                message: Some(message),
                ..Default::default()
            },
            Raw::Object {
                code,
                decline_code,
                message,
                error_type,
            } => StripePaymentError {
                code,
                decline_code,
                message,
                error_type,
            },
        })
    }
}

/// Shape shared between PaymentIntent and SetupIntent.
#[derive(Clone, Debug, Deserialize)]
pub struct StripeNextAction {
    #[serde(rename = "type")]
    pub action_type: String,
    pub redirect_to_url: Option<StripeRedirectToUrl>,
    /// Opaque blob consumed by Stripe's client SDK (3DS modal, etc.); surfaced verbatim.
    pub use_stripe_sdk: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StripeRedirectToUrl {
    pub url: Option<String>,
    pub return_url: Option<String>,
}

#[derive(Clone, Default, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StripePaymentStatus {
    Succeeded,
    Failed,
    #[default]
    Processing,
    #[serde(rename = "requires_action")]
    RequiresCustomerAction,
    RequiresPaymentMethod,
    RequiresConfirmation,
    Canceled,
    RequiresCapture,
    Chargeable,
    Consumed,
    Pending,
}

#[derive(Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StripeCaptureMethod {
    Manual,
    #[default]
    Automatic,
    AutomaticAsync,
}

#[async_trait::async_trait]
pub trait PaymentIntentApi {
    async fn create_payment_intent(
        &self,
        params: PaymentIntentRequest,
        secret_key: &SecretString,
        idempotency_key: String,
    ) -> Result<StripePaymentIntent, StripeError>;

    /// Used by reconciliation to recover from outbound calls that timed out before
    /// the response was recorded: the local transaction stays `Pending` and we poll.
    async fn get_payment_intent(
        &self,
        id: &str,
        secret_key: &SecretString,
    ) -> Result<StripePaymentIntent, StripeError>;
}

#[async_trait::async_trait]
impl PaymentIntentApi for StripeClient {
    async fn create_payment_intent(
        &self,
        params: PaymentIntentRequest,
        secret_key: &SecretString,
        idempotency_key: String,
    ) -> Result<StripePaymentIntent, StripeError> {
        self.post_form(
            "/payment_intents",
            params,
            secret_key,
            idempotency_key,
            RetryStrategy::default(),
        )
        .await
    }

    async fn get_payment_intent(
        &self,
        id: &str,
        secret_key: &SecretString,
    ) -> Result<StripePaymentIntent, StripeError> {
        self.get(
            &format!("/payment_intents/{id}"),
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

    /// Regression for a `webhook_in` dead-letter: Stripe sends `last_payment_error`
    /// as an object whose nested `payment_method` is itself an expanded object.
    /// Typing the field as `Option<String>` made this fail with
    /// `invalid type: map, expected a string`.
    #[test]
    fn payment_failed_event_with_last_payment_error_object_parses() {
        let body = r#"{
  "id": "evt_3TsxL9G04xPnL1XD0yqbJ9gM",
  "object": "event",
  "api_version": "2024-04-10",
  "created": 1784000943,
  "data": {
    "object": {
      "id": "pi_3TsxL9G04xPnL1XD0NYXltWg",
      "object": "payment_intent",
      "amount": 1000,
      "amount_capturable": 0,
      "amount_details": { "tip": {} },
      "amount_received": 0,
      "application": null,
      "capture_method": "automatic",
      "client_secret": "pi_3TsxL9G04xPnL1XD0NYXltWg_secret_gFicn3fZgxpm87lfPQ3LXtEM7",
      "confirmation_method": "automatic",
      "created": 1784000943,
      "currency": "usd",
      "customer": "cus_UsiljJoGmjmP1Z",
      "description": null,
      "invoice": null,
      "last_payment_error": {
        "advice_code": "try_again_later",
        "charge": "ch_3TsxL9G04xPnL1XD0208JB7A",
        "code": "card_declined",
        "decline_code": "generic_decline",
        "doc_url": "https://stripe.com/docs/error-codes/card-declined",
        "message": "Your card was declined.",
        "network_decline_code": "01",
        "payment_method": {
          "id": "pm_1TsxL8G04xPnL1XDIZcJMzbb",
          "object": "payment_method",
          "billing_details": {
            "address": { "country": "US", "postal_code": "42424" },
            "name": "e2e-declined-1784000738041"
          },
          "card": {
            "brand": "visa",
            "checks": { "address_postal_code_check": "pass", "cvc_check": "pass" },
            "country": "US",
            "exp_month": 12,
            "exp_year": 2034,
            "fingerprint": "reibGveE7ooGzAln",
            "funding": "credit",
            "last4": "0341",
            "networks": { "available": ["visa"], "preferred": null }
          },
          "created": 1784000942,
          "customer": "cus_UsiljJoGmjmP1Z",
          "livemode": false,
          "metadata": {},
          "type": "card"
        },
        "type": "card_error"
      },
      "latest_charge": "ch_3TsxL9G04xPnL1XD0208JB7A",
      "livemode": false,
      "metadata": {
        "meteroid.transaction_id": "pay_69ay84GixNCIQGuRHr4LAC",
        "meteroid.tenant_id": "ten_2He1Pg8DoQ31a56yywzg4i"
      },
      "next_action": null,
      "payment_method": null,
      "payment_method_types": ["card"],
      "status": "requires_payment_method"
    }
  },
  "livemode": false,
  "pending_webhooks": 1,
  "request": {
    "id": "req_K8olLxhuT2X4D5",
    "idempotency_key": "charge:pay_69ay84GixNCIQGuRHr4LAC"
  },
  "type": "payment_intent.payment_failed"
}"#;

        let event = StripeWebhook::parse_event(body).expect("parse payment_failed event");
        assert_eq!(event.event_type, "payment_intent.payment_failed");

        let intent = match event.data.object {
            EventObject::PaymentIntent(intent) => intent,
            _ => panic!("expected a PaymentIntent event object"),
        };
        assert_eq!(intent.status, StripePaymentStatus::RequiresPaymentMethod);

        let err = intent
            .last_payment_error
            .expect("last_payment_error should parse as an object");
        assert_eq!(err.code.as_deref(), Some("card_declined"));
        assert_eq!(err.decline_code.as_deref(), Some("generic_decline"));
        assert_eq!(err.error_type.as_deref(), Some("card_error"));
        assert_eq!(err.to_message(), "Your card was declined.");
    }

    /// Defensive: Stripe documents this field as an object, but a bare string
    /// must still parse (folded into `message`) rather than dead-letter.
    #[test]
    fn last_payment_error_tolerates_bare_string() {
        let err: StripePaymentError =
            serde_json::from_str(r#""Your card was declined.""#).expect("parse string error");
        assert_eq!(err.message.as_deref(), Some("Your card was declined."));
        assert_eq!(err.to_message(), "Your card was declined.");
    }
}
