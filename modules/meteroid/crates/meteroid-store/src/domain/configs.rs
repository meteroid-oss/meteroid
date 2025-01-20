use crate::domain::enums::InvoicingProviderEnum;
use crate::errors::StoreError;
use crate::StoreResult;
use chrono::NaiveDateTime;
use diesel_models::configs::{ProviderConfigRow, ProviderConfigRowNew};
use error_stack::ResultExt;
use o2o::o2o;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebhookSecurity {
    pub secret: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiSecurity {
    pub api_key: String,
}

#[derive(Clone, Debug)]
pub struct ProviderConfig {
    pub id: Uuid,
    pub created_at: NaiveDateTime,
    pub tenant_id: Uuid,
    pub invoicing_provider: InvoicingProviderEnum,
    pub enabled: bool,
    pub webhook_security: WebhookSecurity,
    pub api_security: ApiSecurity,
}

impl ProviderConfig {
    pub fn from_row(key: &SecretString, row: ProviderConfigRow) -> StoreResult<ProviderConfig> {
        let enc_wh_sec: WebhookSecurity =
            serde_json::from_value(row.webhook_security).map_err(|e| {
                StoreError::SerdeError("Failed to deserialize webhook_security".to_string(), e)
            })?;

        let enc_api_sec: ApiSecurity = serde_json::from_value(row.api_security).map_err(|e| {
            StoreError::SerdeError("Failed to deserialize api_security".to_string(), e)
        })?;

        let wh_sec = WebhookSecurity {
            secret: crate::crypt::decrypt(key, enc_wh_sec.secret.as_str())
                .change_context(StoreError::CryptError(
                    "webhook_security decryption error".into(),
                ))?
                .expose_secret()
                .clone(),
        };

        let api_sec = ApiSecurity {
            api_key: crate::crypt::decrypt(key, enc_api_sec.api_key.as_str())
                .change_context(StoreError::CryptError(
                    "api_security decryption error".into(),
                ))?
                .expose_secret()
                .clone(),
        };

        Ok(ProviderConfig {
            id: row.id,
            created_at: row.created_at,
            tenant_id: row.tenant_id,
            invoicing_provider: row.invoicing_provider.into(),
            enabled: row.enabled,
            webhook_security: wh_sec,
            api_security: api_sec,
        })
    }
}

#[derive(Clone, Debug, o2o)]
#[from_owned(ProviderConfigRow)]
pub struct ProviderConfigMeta {
    pub id: Uuid,
    pub tenant_id: Uuid,
    #[map(~.into())]
    pub invoicing_provider: InvoicingProviderEnum,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct ProviderConfigNew {
    pub tenant_id: Uuid,
    pub invoicing_provider: InvoicingProviderEnum,
    pub enabled: bool,
    pub webhook_security: WebhookSecurity,
    pub api_security: ApiSecurity,
}

impl ProviderConfigNew {
    pub fn to_row(&self, key: &SecretString) -> StoreResult<ProviderConfigRowNew> {
        let wh_sec_enc = WebhookSecurity {
            secret: crate::crypt::encrypt(key, self.webhook_security.secret.as_str())
                .change_context(StoreError::CryptError(
                    "webhook_security encryption error".into(),
                ))?,
        };

        let api_sec_enc = ApiSecurity {
            api_key: crate::crypt::encrypt(key, self.api_security.api_key.as_str())
                .change_context(StoreError::CryptError(
                    "api_security encryption error".into(),
                ))?,
        };

        let wh_sec = serde_json::to_value(&wh_sec_enc).map_err(|e| {
            StoreError::SerdeError("Failed to serialize webhook_security".to_string(), e)
        })?;

        let api_sec = serde_json::to_value(&api_sec_enc).map_err(|e| {
            StoreError::SerdeError("Failed to serialize api_security".to_string(), e)
        })?;

        Ok(ProviderConfigRowNew {
            id: Uuid::now_v7(),
            tenant_id: self.tenant_id,
            invoicing_provider: self.invoicing_provider.clone().into(),
            enabled: self.enabled,
            webhook_security: wh_sec,
            api_security: api_sec,
        })
    }
}
