use crate::client::MollieClient;
use crate::error::MollieError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::HashMap;

/// `POST /v2/customers` — every field is optional at Mollie.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCustomer {
    pub name: Option<String>,
    pub email: Option<String>,
    pub locale: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MollieCustomer {
    pub id: String,
    pub mode: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    #[serde(default, deserialize_with = "crate::payments::deserialize_metadata")]
    pub metadata: HashMap<String, String>,
}

impl MollieClient {
    pub async fn create_customer(
        &self,
        params: CreateCustomer,
        api_key: &SecretString,
        idempotency_key: &str,
    ) -> Result<MollieCustomer, MollieError> {
        self.post_json(
            "/customers",
            params,
            api_key,
            Some(idempotency_key),
            RetryStrategy::default(),
        )
        .await
    }

    pub async fn get_customer(
        &self,
        customer_id: &str,
        api_key: &SecretString,
    ) -> Result<MollieCustomer, MollieError> {
        self.get(
            &format!("/customers/{customer_id}"),
            api_key,
            RetryStrategy::default(),
        )
        .await
    }
}
