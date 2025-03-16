use crate::domain::oauth::{OauthTokens, OauthUser, OauthVerifier, OauthVerifierData};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use async_trait::async_trait;
use chrono::Utc;
use diesel_models::oauth_verifiers::OauthVerifierRow;
use error_stack::{Report, ResultExt};
use meteroid_oauth::model::OauthProvider;
use secrecy::{ExposeSecret, SecretString};
use std::ops::Add;

#[async_trait]
pub trait OauthInterface {
    async fn oauth_auth_provider_url(
        &self,
        provider: OauthProvider,
        data: OauthVerifierData,
    ) -> StoreResult<SecretString>;

    async fn oauth_get_user(
        &self,
        provider: OauthProvider,
        code: SecretString,
        state: SecretString,
    ) -> StoreResult<OauthUser>;

    async fn oauth_exchange_code(
        &self,
        provider: OauthProvider,
        code: SecretString,
        state: SecretString,
    ) -> StoreResult<OauthTokens>;
}

#[async_trait]
impl OauthInterface for Store {
    async fn oauth_auth_provider_url(
        &self,
        provider: OauthProvider,
        data: OauthVerifierData,
    ) -> StoreResult<SecretString> {
        let callback_url = self
            .oauth
            .for_provider(provider)
            .ok_or(Report::new(StoreError::OauthError(
                "Provider not configured".to_string(),
            )))?
            .callback_url();

        let verifier = OauthVerifier {
            csrf_token: callback_url.csrf_token,
            pkce_verifier: callback_url.pkce_verifier,
            data,
        };

        let row = verifier.to_row(&self.settings.crypt_key)?;

        let mut conn = self.get_conn().await?;

        row.insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(callback_url.url)
    }

    async fn oauth_get_user(
        &self,
        provider: OauthProvider,
        code: SecretString,
        state: SecretString,
    ) -> StoreResult<OauthUser> {
        let verifiers = get_verifier(self, state).await?;

        let user = self
            .oauth
            .for_provider(provider)
            .ok_or(Report::new(StoreError::OauthError(
                "Provider not configured".to_string(),
            )))?
            .get_user_info(code, verifiers.pkce_verifier)
            .await
            .change_context(StoreError::OauthError(
                "Failed to fetch oauth user".to_owned(),
            ))?;

        Ok(OauthUser {
            user,
            verifier_data: verifiers.data,
        })
    }

    async fn oauth_exchange_code(
        &self,
        provider: OauthProvider,
        code: SecretString,
        state: SecretString,
    ) -> StoreResult<OauthTokens> {
        let verifiers = get_verifier(self, state).await?;

        let tokens = self
            .oauth
            .for_provider(provider)
            .ok_or(Report::new(StoreError::OauthError(
                "Provider not configured".to_string(),
            )))?
            .exchange_code(code, verifiers.pkce_verifier)
            .await
            .change_context(StoreError::OauthError("Failed to exchange code".to_owned()))?;

        Ok(OauthTokens {
            tokens,
            verifier_data: verifiers.data,
        })
    }
}

async fn get_verifier(store: &Store, state: SecretString) -> StoreResult<OauthVerifier> {
    let verifier_ttl = chrono::Duration::minutes(10);

    // todo: we should probably migrate verifiers storage to Redis
    let pool = store.pool.clone();
    tokio::spawn(async move {
        let mut conn = pool.get().await.expect("failed to get connection");

        OauthVerifierRow::delete_expired(&mut conn, Utc::now().add(verifier_ttl).naive_utc())
            .await
            .map_err(Into::<Report<StoreError>>::into)
    });

    let mut conn = store.get_conn().await?;
    let verifiers = OauthVerifierRow::delete_by_csrf_token(&mut conn, state.expose_secret())
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    if verifiers.created_at + verifier_ttl < Utc::now().naive_utc() {
        return Err(StoreError::OauthError("expired verifier".into()).into());
    }

    OauthVerifier::from_row(verifiers, store.settings.crypt_key.clone())
}
