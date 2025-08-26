use crate::crypt::{decrypt, encrypt};
use crate::domain::connectors::ConnectorMeta;
use crate::errors::StoreError;
use crate::{StoreResult, json_value_serde};
use chrono::Utc;
use common_domain::ids::TenantId;
use diesel_models::oauth_verifiers::OauthVerifierRow;
use error_stack::ResultExt;
use meteroid_oauth::model::{OAuthTokens, OAuthUser};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub struct OauthVerifier {
    pub csrf_token: SecretString,
    pub pkce_verifier: SecretString,
    pub data: OauthVerifierData,
}

impl OauthVerifier {
    fn enc(key: &SecretString, raw: &str) -> StoreResult<String> {
        encrypt(key, raw).change_context(StoreError::CryptError("encryption error".into()))
    }

    fn dec(key: &SecretString, enc: &str) -> StoreResult<SecretString> {
        decrypt(key, enc).change_context(StoreError::CryptError("decryption error".into()))
    }

    pub fn to_row(&self, crypt_key: &SecretString) -> StoreResult<OauthVerifierRow> {
        Ok(OauthVerifierRow {
            id: Uuid::now_v7(),
            csrf_token: self.csrf_token.expose_secret().to_string(),
            pkce_verifier: Self::enc(crypt_key, self.pkce_verifier.expose_secret())?,
            data: Some((&self.data).try_into()?),
            created_at: Utc::now().naive_utc(),
        })
    }

    pub fn from_row(row: OauthVerifierRow, crypt_key: SecretString) -> StoreResult<Self> {
        Ok(OauthVerifier {
            csrf_token: SecretString::new(row.csrf_token),
            pkce_verifier: Self::dec(&crypt_key, &row.pkce_verifier)?,
            data: row
                .data
                .ok_or_else(|| StoreError::InvalidArgument("empty data field".into()))?
                .try_into()?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OauthVerifierData {
    SignIn(SignInData),
    ConnectHubspot(ConnectHubspotData),
    ConnectPennylane(ConnectPennylaneData),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignInData {
    pub is_signup: bool,
    pub invite_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectHubspotData {
    pub tenant_id: TenantId,
    pub auto_sync: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectPennylaneData {
    pub tenant_id: TenantId,
}

json_value_serde!(OauthVerifierData);

pub struct OauthConnected {
    pub connector: ConnectorMeta,
}

pub struct OauthConnection {
    pub user: OAuthUser,
    pub tokens: OAuthTokens,
    pub verifier_data: OauthVerifierData,
}
