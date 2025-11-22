use crate::client::StripeClient;
use crate::error::StripeError;
use crate::request::RetryStrategy;
use crate::setup_intents::StripeMandateRequest;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;
use crate::setup_intents::StripePaymentMethodType;

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
    pub next_action: Option<String>, // should not happen as we're offline ?
    pub livemode: bool,
    pub status: StripePaymentStatus,
    pub last_payment_error: Option<String>,
    pub metadata: HashMap<String, String>,
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
}
