use crate::client::StripeClient;
use crate::error::StripeError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Clone, Debug, serde::Serialize)]
pub struct CreateCustomer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<OptionalFieldsAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coupon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<std::collections::HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_locales: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shipping: Option<CustomerShipping>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate: Option<bool>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct OptionalFieldsAddress {
    /// City, district, suburb, town, or village.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    /// Two-letter country code ([ISO 3166-1 alpha-2](https://en.wikipedia.org/wiki/ISO_3166-1_alpha-2)).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// Address line 1 (e.g., street, PO Box, or company name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line1: Option<String>,
    /// Address line 2 (e.g., apartment, suite, unit, or building).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line2: Option<String>,
    /// ZIP or postal code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postal_code: Option<String>,
    /// State, county, province, or region.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct CustomerShipping {
    /// Customer shipping address.
    pub address: OptionalFieldsAddress,
    /// Customer name.
    pub name: String,
    /// Customer phone (including extension).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Customer {
    pub id: String,
}

#[async_trait::async_trait]
pub trait CustomerApi {
    async fn create_customer(
        &self,
        params: CreateCustomer,
        secret_key: &SecretString,
        idempotency_key: String,
    ) -> Result<Customer, StripeError>;
}

#[async_trait::async_trait]
impl CustomerApi for StripeClient {
    async fn create_customer(
        &self,
        params: CreateCustomer,
        secret_key: &SecretString,
        idempotency_key: String,
    ) -> Result<Customer, StripeError> {
        self.post_form(
            "/customers",
            params,
            secret_key,
            idempotency_key,
            RetryStrategy::default(),
        )
        .await
    }
}
