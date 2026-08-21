use crate::StoreResult;
use crate::domain::connectors::GocardlessSensitiveData;
use crate::errors::StoreError;
use crate::services::ServicesEdge;
use error_stack::Report;
use gocardless_client::client::GoCardlessClient;
use gocardless_client::customers::CustomerApi;
use secrecy::SecretString;

impl ServicesEdge {
    /// Cheap authenticated GET against the GoCardless API to catch a
    /// mistyped/wrong-environment token before we persist it, mirroring
    /// `get_stripe_account_id`.
    pub async fn validate_gocardless_credentials(
        &self,
        gocardless_data: &GocardlessSensitiveData,
        sandbox: bool,
    ) -> StoreResult<()> {
        let client = if sandbox {
            GoCardlessClient::from_sandbox()
        } else {
            GoCardlessClient::new()
        };
        let token = SecretString::from(gocardless_data.access_token.clone());

        client
            .validate_credentials(&token)
            .await
            .map_err(|err| Report::new(err).change_context(StoreError::PaymentProviderError))?;

        Ok(())
    }
}
