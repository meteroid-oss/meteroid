use crate::StoreResult;
use crate::domain::connectors::StripeSensitiveData;
use crate::errors::StoreError;
use crate::services::ServicesEdge;
use error_stack::Report;
use secrecy::SecretString;
use stripe_client::accounts::AccountsApi;

impl ServicesEdge {
    pub async fn get_stripe_account_id(
        &self,
        stripe_data: &StripeSensitiveData,
    ) -> StoreResult<String> {
        // we test with the account api, and fail if we cannot reach it
        let secret = &SecretString::new(stripe_data.api_secret_key.clone());
        let account = self
            .services
            .stripe
            .get_account(secret)
            .await
            .map_err(|err| Report::new(err).change_context(StoreError::PaymentProviderError))?;

        Ok(account.id)
    }
}
