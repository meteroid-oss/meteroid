use crate::client::MollieClient;
use crate::error::MollieError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::Deserialize;

/// Lightest call that accepts an API key; used as the connect-time credential check.
#[derive(Clone, Debug, Deserialize)]
pub struct MethodList {}

impl MollieClient {
    pub async fn list_methods(&self, api_key: &SecretString) -> Result<MethodList, MollieError> {
        self.get("/methods", api_key, RetryStrategy::NoRetry).await
    }
}
