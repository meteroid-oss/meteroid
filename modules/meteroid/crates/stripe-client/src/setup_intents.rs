use crate::client::StripeClient;
use crate::error::StripeError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "mandate_data[customer_acceptance][type]"
)]
pub enum StripeMandateType {
    Online {
        #[serde(rename = "mandate_data[customer_acceptance][online][ip_address]")]
        ip_address: String, // TODO pii
        #[serde(rename = "mandate_data[customer_acceptance][online][user_agent]")]
        user_agent: String,
    },
    Offline,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StripeMandateRequest {
    #[serde(flatten)]
    mandate_type: StripeMandateType,
}

// setup intents are used to create a payment method that can be used to create a payment intent
#[derive(Clone, Debug, Serialize)]
pub struct CreateSetupIntent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer: Option<String>,
    #[serde(flatten)]
    pub setup_mandate_details: Option<StripeMandateRequest>,
    // Connect only
    // #[serde(skip_serializing_if = "Option::is_none")]
    // on_behalf_of: Option<String>,
    // payment_method_options : should we allow more customization here ?
    // livemode
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_method_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<CreateSetupIntentUsage>,
}

#[derive(Clone, Debug, Serialize)]
pub enum CreateSetupIntentUsage {
    #[serde(rename = "off_session")]
    OffSession,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SetupIntent {
    pub id: String,
    pub client_secret: String,
    pub created: i64,
    pub customer: Option<String>,
    pub livemode: bool,
    pub payment_method_types: Vec<String>,
    pub status: String,
    pub usage: String,
}

#[async_trait::async_trait]
pub trait SetupIntentApi {
    async fn create_setup_intent(
        &self,
        params: CreateSetupIntent,
        secret_key: &SecretString,
        idempotency_key: String,
    ) -> Result<SetupIntent, StripeError>;
}

#[async_trait::async_trait]
impl SetupIntentApi for StripeClient {
    async fn create_setup_intent(
        &self,
        params: CreateSetupIntent,
        secret_key: &SecretString,
        idempotency_key: String,
    ) -> Result<SetupIntent, StripeError> {
        self.post_form(
            "/setup_intents",
            params,
            secret_key,
            idempotency_key,
            RetryStrategy::default(),
        )
        .await
    }
}
