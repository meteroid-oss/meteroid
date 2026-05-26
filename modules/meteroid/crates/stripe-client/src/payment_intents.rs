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
    /// Populated by Stripe when the customer must complete an additional step
    /// (3DS, microdeposit verification, bank app SCA). Off-session charges that
    /// trigger SCA come back with status `requires_action` and `next_action`
    /// describing what the portal needs to surface.
    pub next_action: Option<StripeNextAction>,
    pub livemode: bool,
    /// Returned on PaymentIntent creation. The on-session portal needs it to
    /// complete a `requires_action` charge via Stripe.js `handleNextAction`.
    pub client_secret: Option<String>,
    pub status: StripePaymentStatus,
    /// Stripe sends this as a nested object (code, message, decline_code,
    /// payment_method, etc.). Kept as opaque JSON because the variant set is
    /// large and we only display a flattened message downstream.
    pub last_payment_error: Option<serde_json::Value>,
    pub metadata: HashMap<String, String>,
}

/// Describes what the customer must do next to complete a payment / setup
/// intent. Shape is shared between PaymentIntent and SetupIntent.
#[derive(Clone, Debug, Deserialize)]
pub struct StripeNextAction {
    #[serde(rename = "type")]
    pub action_type: String,
    pub redirect_to_url: Option<StripeRedirectToUrl>,
    /// Stripe SDK-driven flows (3DS modal, etc.) attach an opaque blob here
    /// that the client SDK consumes; we surface it verbatim if present.
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

    /// Fetch a payment intent by id. Used by the reconciliation worker to
    /// recover from outbound calls that timed out before we could record the
    /// response — the local transaction stays `Pending` and we poll the
    /// provider to find out what actually happened.
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
