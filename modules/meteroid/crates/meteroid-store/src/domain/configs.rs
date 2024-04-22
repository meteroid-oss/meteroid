use crate::domain::enums::InvoicingProviderEnum;
use crate::errors::StoreError;
use crate::StoreResult;
use chrono::NaiveDateTime;
use error_stack::ResultExt;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebhookSecurity {
    pub secret: String,
}

impl WebhookSecurity {
    pub fn encrypted(&self, key: &SecretString) -> StoreResult<WebhookSecurity> {
        let encrypted = crate::crypt::encrypt(key, self.secret.as_str())
            .change_context(StoreError::CryptError("encryption error".into()))?;
        Ok(WebhookSecurity { secret: encrypted })
    }

    pub fn decrypted(&self, key: &SecretString) -> StoreResult<WebhookSecurity> {
        let decrypted = crate::crypt::decrypt(key, self.secret.as_str())
            .change_context(StoreError::CryptError("decryption error".into()))?;
        Ok(WebhookSecurity {
            secret: decrypted.expose_secret().clone(),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiSecurity {
    pub api_key: String,
}

impl ApiSecurity {
    pub fn encrypted(&self, key: &SecretString) -> StoreResult<ApiSecurity> {
        let crypted = crate::crypt::encrypt(key, self.api_key.as_str())
            .change_context(StoreError::CryptError("encryption error".into()))?;
        Ok(ApiSecurity { api_key: crypted })
    }

    pub fn decrypted(&self, key: &SecretString) -> StoreResult<ApiSecurity> {
        let decrypted = crate::crypt::decrypt(key, self.api_key.as_str())
            .change_context(StoreError::CryptError("decryption error".into()))?;
        Ok(ApiSecurity {
            api_key: decrypted.expose_secret().clone(),
        })
    }
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
    pub fn decrypted(&self, key: &SecretString) -> StoreResult<ProviderConfig> {
        Ok(ProviderConfig {
            api_security: self.api_security.decrypted(key)?,
            webhook_security: self.webhook_security.decrypted(key)?,
            tenant_id: self.tenant_id,
            invoicing_provider: self.invoicing_provider.clone(),
            enabled: self.enabled,
            id: self.id,
            created_at: self.created_at,
        })
    }
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
    pub fn encrypted(&self, key: &SecretString) -> StoreResult<ProviderConfigNew> {
        Ok(ProviderConfigNew {
            api_security: self.api_security.encrypted(key)?,
            webhook_security: self.webhook_security.encrypted(key)?,
            tenant_id: self.tenant_id,
            invoicing_provider: self.invoicing_provider.clone(),
            enabled: self.enabled,
        })
    }
}

impl TryInto<diesel_models::configs::ProviderConfigNew> for ProviderConfigNew {
    type Error = StoreError;

    fn try_into(self) -> Result<diesel_models::configs::ProviderConfigNew, StoreError> {
        let wh_sec = serde_json::to_value(&self.webhook_security).map_err(|e| {
            StoreError::SerdeError("Failed to serialize webhook_security".to_string(), e)
        })?;

        let api_sec = serde_json::to_value(&self.api_security).map_err(|e| {
            StoreError::SerdeError("Failed to serialize api_security".to_string(), e)
        })?;

        Ok(diesel_models::configs::ProviderConfigNew {
            id: Uuid::now_v7(),
            tenant_id: self.tenant_id,
            invoicing_provider: self.invoicing_provider.into(),
            enabled: self.enabled,
            webhook_security: wh_sec,
            api_security: api_sec,
        })
    }
}

impl TryFrom<diesel_models::configs::ProviderConfig> for ProviderConfig {
    type Error = StoreError;

    fn try_from(
        value: diesel_models::configs::ProviderConfig,
    ) -> Result<ProviderConfig, StoreError> {
        let wh_sec: WebhookSecurity =
            serde_json::from_value(value.webhook_security).map_err(|e| {
                StoreError::SerdeError("Failed to deserialize webhook_security".to_string(), e)
            })?;

        let api_sec: ApiSecurity = serde_json::from_value(value.api_security).map_err(|e| {
            StoreError::SerdeError("Failed to deserialize api_security".to_string(), e)
        })?;

        Ok(ProviderConfig {
            id: value.id,
            created_at: value.created_at,
            tenant_id: value.tenant_id,
            invoicing_provider: value.invoicing_provider.into(),
            enabled: value.enabled,
            webhook_security: wh_sec,
            api_security: api_sec,
        })
    }
}
