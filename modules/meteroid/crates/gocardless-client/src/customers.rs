//! Customers resource.
//!
//! GoCardless wraps both request and response bodies in a top-level resource key
//! (`{ "customers": { ... } }`); the `*Envelope` structs model that.

use crate::client::GoCardlessClient;
use crate::error::GoCardlessError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize)]
pub struct CreateCustomer {
    pub email: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub company_name: Option<String>,
    pub language: Option<String>,
    pub phone_number: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub address_line3: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub postal_code: Option<String>,
    /// ISO 3166-1 alpha-2.
    pub country_code: Option<String>,
    /// Up to 3 keys; key max 50 chars, value max 500 chars.
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
struct CreateCustomerEnvelope<'a> {
    customers: &'a CreateCustomer,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Customer {
    /// `CU...` prefix.
    pub id: String,
    pub created_at: Option<String>,
    pub email: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub company_name: Option<String>,
    pub language: Option<String>,
    pub phone_number: Option<String>,
    pub country_code: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct CustomerEnvelope {
    customers: Customer,
}

#[async_trait::async_trait]
pub trait CustomerApi {
    async fn create_customer(
        &self,
        params: CreateCustomer,
        access_token: &SecretString,
        idempotency_key: &str,
    ) -> Result<Customer, GoCardlessError>;
}

#[async_trait::async_trait]
impl CustomerApi for GoCardlessClient {
    async fn create_customer(
        &self,
        params: CreateCustomer,
        access_token: &SecretString,
        idempotency_key: &str,
    ) -> Result<Customer, GoCardlessError> {
        let envelope = CreateCustomerEnvelope { customers: &params };
        let resp: CustomerEnvelope = self
            .post(
                "/customers",
                envelope,
                access_token,
                Some(idempotency_key),
                RetryStrategy::default(),
            )
            .await?;
        Ok(resp.customers)
    }
}
