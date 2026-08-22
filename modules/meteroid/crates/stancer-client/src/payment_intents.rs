use crate::client::StancerClient;
use crate::error::StancerError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;

/// Stancer has no client-side tokenization library: the hosted-page flow is
/// the only PCI-safe way to collect a card — the customer types it directly
/// into Stancer's page at `url`, so raw card data never transits Meteroid.
///
/// A pure "save this card" step (no charge) uses `amount: 0` and
/// `capture: false` — accepted by the API shape, though a genuine $0 network
/// authorization is unverified without a live card test.
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
    /// Any unmodeled status. Must never fail the intent parse; consumers
    /// treat it as not-yet-complete / not-cancelable, never as done.
    #[serde(other)]
    Unknown,
}

/// `PATCH /v2/payment_intents/{id}`: omitted fields are left untouched.
/// Used to bake the intent's own id into `return_url` after creation (the id
/// is only known once the intent exists).
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

    /// Cancel a payment intent so its hosted page can never capture money —
    /// the spec's only cancellation route (PATCH has no `status` field). A 422
    /// means not cancelable in its current state (e.g. already captured).
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

    /// An unmodeled status must deserialize to `Unknown`, never fail the
    /// whole intent parse (which would wedge every in-flight attempt).
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
