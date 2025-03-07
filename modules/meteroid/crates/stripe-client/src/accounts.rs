use crate::client::StripeClient;
use crate::error::StripeError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Account {
    pub id: String,
}

#[async_trait::async_trait]
pub trait AccountsApi {
    async fn get_account(&self, secret_key: &SecretString) -> Result<Account, StripeError>;
}

#[async_trait::async_trait]
impl AccountsApi for StripeClient {
    async fn get_account(&self, secret_key: &SecretString) -> Result<Account, StripeError> {
        self.get("/account", secret_key, RetryStrategy::default())
            .await
    }
}
