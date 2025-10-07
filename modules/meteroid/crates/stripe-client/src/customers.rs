use crate::client::StripeClient;
use crate::error::StripeError;
use crate::request::RetryStrategy;
use common_domain::country::CountryCode;
use secrecy::SecretString;
use serde::Deserialize;
use serde_with::skip_serializing_none;

#[skip_serializing_none]
#[derive(Clone, Debug, serde::Serialize)]
pub struct CreateCustomer {
    pub address: Option<OptionalFieldsAddress>,
    pub coupon: Option<String>,
    pub description: Option<String>,
    pub email: Option<String>,
    pub metadata: Option<std::collections::HashMap<String, String>>,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub preferred_locales: Option<Vec<String>>,
    pub shipping: Option<CustomerShipping>,
    pub source: Option<String>,
    pub validate: Option<bool>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, serde::Serialize)]
pub struct OptionalFieldsAddress {
    /// City, district, suburb, town, or village.
    pub city: Option<String>,
    /// Two-letter country code ([ISO 3166-1 alpha-2](https://en.wikipedia.org/wiki/ISO_3166-1_alpha-2)).
    pub country: Option<CountryCode>,
    /// Address line 1 (e.g., street, PO Box, or company name).
    pub line1: Option<String>,
    /// Address line 2 (e.g., apartment, suite, unit, or building).
    pub line2: Option<String>,
    /// ZIP or postal code.
    pub postal_code: Option<String>,
    /// State, county, province, or region.
    pub state: Option<String>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, serde::Serialize)]
pub struct CustomerShipping {
    /// Customer shipping address.
    pub address: OptionalFieldsAddress,
    /// Customer name.
    pub name: String,
    /// Customer phone (including extension).
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
