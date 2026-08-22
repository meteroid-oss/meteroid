use crate::client::StancerClient;
use crate::error::StancerError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;

/// Stancer has no client-side tokenization library (unlike Stripe.js): the
/// only PCI-safe way to collect a card is this hosted-page flow. The
/// response's `url` is embedded in an iframe on Meteroid's frontend; the
/// customer types their card directly into Stancer's page at that origin,
/// so raw card data never transits Meteroid's frontend or backend.
///
/// For a pure "save this card" step (no charge), use `amount: 0` and
/// `capture: false` — confirmed accepted by the API shape. Whether a real
/// card network processes a genuine $0 authorization to completion is
/// unverified without a live browser + real card test; if that turns out
/// to be rejected in practice, fall back to a nominal amount with
/// `capture: false` followed by canceling the resulting payment so no
/// funds settle.
#[skip_serializing_none]
#[derive(Debug, Serialize)]
pub struct CreatePaymentIntent {
    pub amount: i64,
    pub currency: String,
    pub customer: Option<String>,
    pub methods_allowed: Vec<StancerPaymentMethod>,
    pub capture: bool,
    pub return_url: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
    pub threeds: Option<ThreeDsMode>,
    pub order_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StancerPaymentMethod {
    Card,
    Sepa,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreeDsMode {
    Required,
    None,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StancerPaymentIntent {
    pub id: String,
    pub customer: Option<String>,
    /// The resulting charge, once the hosted page completes.
    pub payment: Option<String>,
    /// The resulting saved card id, once the hosted page completes — this
    /// is what gets persisted as the customer's payment method.
    pub card: Option<String>,
    pub amount: i64,
    pub currency: String,
    pub status: PaymentIntentStatus,
    pub metadata: Option<HashMap<String, String>>,
    /// Hosted payment page url to embed in an iframe (or redirect to).
    pub url: String,
    pub capture: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentIntentStatus {
    #[default]
    RequirePaymentMethod,
    RequireAuthentication,
    RequireAuthorization,
    Authorized,
    Processing,
    Captured,
    Canceled,
    Unpaid,
    /// Any unmodeled status. Must never fail the intent parse (that would
    /// wedge completion and close-out); consumers treat it as
    /// not-yet-complete / not-cancelable, never as done.
    #[serde(other)]
    Unknown,
}

/// Partial update of a payment intent (`PATCH /v2/payment_intents/{id}`,
/// spec `PaymentIntentUpdate`). Only the fields Meteroid needs are modeled;
/// omitted fields are left untouched by the API. The port uses it to bake
/// the intent's own id into `return_url` after creation (the id is only
/// known once the intent exists).
#[skip_serializing_none]
#[derive(Debug, Default, Serialize)]
pub struct UpdatePaymentIntent {
    pub return_url: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
}

impl StancerClient {
    pub async fn create_payment_intent(
        &self,
        params: CreatePaymentIntent,
        secret_key: &SecretString,
    ) -> Result<StancerPaymentIntent, StancerError> {
        self.post_json(
            "/payment_intents/",
            params,
            secret_key,
            RetryStrategy::default(),
        )
        .await
    }

    pub async fn get_payment_intent(
        &self,
        payment_intent_id: &str,
        secret_key: &SecretString,
    ) -> Result<StancerPaymentIntent, StancerError> {
        self.get(
            &format!("/payment_intents/{payment_intent_id}"),
            secret_key,
            RetryStrategy::default(),
        )
        .await
    }

    pub async fn update_payment_intent(
        &self,
        payment_intent_id: &str,
        params: UpdatePaymentIntent,
        secret_key: &SecretString,
    ) -> Result<StancerPaymentIntent, StancerError> {
        self.patch_json(
            &format!("/payment_intents/{payment_intent_id}"),
            params,
            secret_key,
            RetryStrategy::default(),
        )
        .await
    }

    /// Cancel a payment intent so its hosted page can never capture money.
    /// `DELETE /v2/payment_intents/{id}` is the spec's cancellation route (the
    /// `PaymentIntentUpdate` PATCH schema has no `status` field); a 200 returns
    /// the intent, whose status set includes `canceled`. A 422 means the intent
    /// is not cancelable in its current state (e.g. already captured).
    pub async fn delete_payment_intent(
        &self,
        payment_intent_id: &str,
        secret_key: &SecretString,
    ) -> Result<StancerPaymentIntent, StancerError> {
        self.delete(
            &format!("/payment_intents/{payment_intent_id}"),
            secret_key,
            RetryStrategy::default(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{PaymentIntentStatus, StancerPaymentIntent};

    fn intent_json(status: &str) -> String {
        format!(
            r#"{{
                "id": "pi_test1",
                "customer": "cust_1",
                "payment": null,
                "card": null,
                "amount": 4200,
                "currency": "eur",
                "status": "{status}",
                "metadata": null,
                "url": "https://payment.stancer.com/pi_test1",
                "capture": true
            }}"#
        )
    }

    /// A status value this client does not model must deserialize to
    /// `Unknown`, never fail the whole intent parse (which would wedge
    /// completion and close-out of every in-flight attempt).
    #[test]
    fn unknown_intent_status_deserializes_to_unknown() {
        let intent: StancerPaymentIntent =
            serde_json::from_str(&intent_json("some_future_status")).expect("parse must not fail");
        assert_eq!(intent.status, PaymentIntentStatus::Unknown);

        // Modeled statuses keep their exact mapping.
        let intent: StancerPaymentIntent =
            serde_json::from_str(&intent_json("captured")).expect("parse ok");
        assert_eq!(intent.status, PaymentIntentStatus::Captured);
        let intent: StancerPaymentIntent =
            serde_json::from_str(&intent_json("canceled")).expect("parse ok");
        assert_eq!(intent.status, PaymentIntentStatus::Canceled);
    }
}
