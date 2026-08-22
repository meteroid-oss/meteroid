use crate::client::StancerClient;
use crate::error::StancerError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

#[skip_serializing_none]
#[derive(Debug, Default, Eq, PartialEq, Serialize)]
pub struct CreateCustomer {
    pub email: Option<String>,
    pub name: Option<String>,
    pub mobile: Option<String>,
    pub date_birth: Option<String>,
    pub legal_id: Option<String>,
    pub country: Option<String>,
    /// Correlates the Stancer customer back to a Meteroid entity — Stancer's
    /// customer resource has no free-form metadata map, this is the only
    /// available slot (≤36 chars, unique).
    pub external_id: Option<String>,
    pub billing_address: Option<String>,
    pub shipping_address: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StancerCustomer {
    pub id: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub mobile: Option<String>,
    pub external_id: Option<String>,
    pub deleted: bool,
}

/// Paginated envelope of `GET /v2/customers/` (spec `CustomerOutList`). Only
/// the entries are consumed; the `range` block is ignored.
#[derive(Clone, Debug, Deserialize)]
pub struct StancerCustomerList {
    pub customers: Vec<StancerCustomer>,
}

impl StancerClient {
    pub async fn create_customer(
        &self,
        params: CreateCustomer,
        secret_key: &SecretString,
    ) -> Result<StancerCustomer, StancerError> {
        self.post_json("/customers/", params, secret_key, RetryStrategy::default())
            .await
    }

    pub async fn get_customer(
        &self,
        customer_id: &str,
        secret_key: &SecretString,
    ) -> Result<StancerCustomer, StancerError> {
        self.get(
            &format!("/customers/{customer_id}"),
            secret_key,
            RetryStrategy::default(),
        )
        .await
    }

    /// `GET /v2/customers/?external_id=…` — resolve the Stancer customer that
    /// carries a given (unique) `external_id`. Used to recover from a retried
    /// `create_customer` whose unique external_id already exists: the conflict
    /// means the customer was created on an earlier attempt — look it up
    /// instead of failing.
    pub async fn list_customers_by_external_id(
        &self,
        external_id: &str,
        secret_key: &SecretString,
    ) -> Result<StancerCustomerList, StancerError> {
        self.get_with_query(
            "/customers/",
            &[("external_id", external_id)],
            secret_key,
            RetryStrategy::default(),
        )
        .await
    }
}
