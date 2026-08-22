use crate::client::StancerClient;
use crate::error::StancerError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;

/// Read-only: cards are only ever created via the payment_intents hosted
/// page (see `payment_intents.rs`), never by Meteroid submitting raw card
/// data. This is used to fetch display metadata (brand/last4/expiry) for a
/// `card_xxx` id right after it's saved, before any `/v2/payments/` charge
/// has happened to surface that data another way.
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
