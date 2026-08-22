use crate::client::StancerClient;
use crate::error::StancerError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;

/// `GET /v2/ping` — the lightest way to validate a secret key is authentic
/// and working, since Stancer has no dedicated "verify key" endpoint.
#[derive(Clone, Debug, Deserialize)]
pub struct StancerPing {
    pub mode: String,
    pub company: String,
    pub account: String,
}

impl StancerClient {
    pub async fn ping(&self, secret_key: &SecretString) -> Result<StancerPing, StancerError> {
        self.get("/ping", secret_key, RetryStrategy::default())
            .await
    }
}
