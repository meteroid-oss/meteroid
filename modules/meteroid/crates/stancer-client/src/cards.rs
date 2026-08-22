use crate::client::StancerClient;
use crate::error::StancerError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;

/// Read-only: cards are only ever created via the hosted page, never by
/// Meteroid submitting raw card data. Used to fetch display metadata
/// (brand/last4/expiry) for a freshly saved `card_xxx` id.
#[derive(Clone, Debug, Deserialize)]
pub struct StancerCard {
    pub id: String,
    pub customer: Option<String>,
    pub brand: Option<String>,
    pub country: Option<String>,
    pub exp_month: u32,
    pub exp_year: u32,
    pub last4: String,
    pub deleted: bool,
}

impl StancerClient {
    pub async fn get_card(
        &self,
        card_id: &str,
        secret_key: &SecretString,
    ) -> Result<StancerCard, StancerError> {
        self.get(
            &format!("/cards/{card_id}"),
            secret_key,
            RetryStrategy::default(),
        )
        .await
    }
}
