use crate::domain::enums::{ConnectorProviderEnum, ConnectorTypeEnum};
use crate::errors::StoreError;
use crate::StoreResult;
use chrono::NaiveDateTime;
use diesel_models::connectors::{ConnectorRow, ConnectorRowNew};
use error_stack::ResultExt;
use o2o::o2o;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Connector {
    pub id: Uuid,
    pub created_at: NaiveDateTime,
    pub tenant_id: Uuid,
    pub alias: String,
    pub connector_type: ConnectorTypeEnum,
    pub provider: ConnectorProviderEnum,
    pub data: Option<ProviderData>,
    // this gets turned into json string then encrypted before storing
    pub sensitive: Option<ProviderSensitiveData>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ProviderData {}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ProviderSensitiveData {
    Stripe(StripeSensitiveData),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StripeSensitiveData {
    pub api_secret_key: String,
    pub webhook_secret: String,
}

impl Connector {
    pub fn from_row(key: &SecretString, row: ConnectorRow) -> StoreResult<Connector> {
        let sensitive = if let Some(s) = row.sensitive {
            let decoded_sensitive_str = crate::crypt::decrypt(key, s.as_str())
                .change_context(StoreError::CryptError("connector decryption error".into()))?
                .expose_secret()
                .clone();
            let sensitive: ProviderSensitiveData = serde_json::from_str(&decoded_sensitive_str)
                .map_err(|e| {
                    StoreError::SerdeError(
                        "Failed to deserialize sensitive connector data".to_string(),
                        e,
                    )
                })?;
            Some(sensitive)
        } else {
            None
        };

        let data = row
            .data
            .map(|d| {
                serde_json::from_value(d).map_err(|e| {
                    StoreError::SerdeError("Failed to deserialize connector data".to_string(), e)
                })
            })
            .transpose()?;

        Ok(Connector {
            id: row.id,
            created_at: row.created_at,
            tenant_id: row.tenant_id,
            alias: row.alias,
            connector_type: row.connector_type.into(),
            provider: row.provider.into(),
            data,
            sensitive,
        })
    }
}

#[derive(Clone, Debug, o2o)]
#[from_owned(ConnectorRow)]
pub struct ConnectorMeta {
    pub id: Uuid,
    pub created_at: NaiveDateTime,
    pub tenant_id: Uuid,
    pub alias: String,
    #[map(~.into())]
    pub connector_type: ConnectorTypeEnum,
    #[map(~.into())]
    pub provider: ConnectorProviderEnum,
}

#[derive(Clone, Debug)]
pub struct ConnectorNew {
    pub tenant_id: Uuid,
    pub alias: String,
    pub connector_type: ConnectorTypeEnum,
    pub provider: ConnectorProviderEnum,
    pub data: Option<ProviderData>,
    pub sensitive: Option<ProviderSensitiveData>,
}

impl ConnectorNew {
    pub fn to_row(&self, key: &SecretString) -> StoreResult<ConnectorRowNew> {
        let sensitive = if let Some(sensitive) = &self.sensitive {
            let sensitive = serde_json::to_string(sensitive).map_err(|e| {
                StoreError::SerdeError("Failed to serialize webhook_security".to_string(), e)
            })?;

            let encoded_sensitive_str = crate::crypt::encrypt(key, sensitive.as_str())
                .change_context(StoreError::CryptError(
                    "webhook_security encryption error".into(),
                ))?;
            Some(encoded_sensitive_str)
        } else {
            None
        };

        let data = match &self.data {
            Some(d) => serde_json::to_value(d)
                .map_err(|e| {
                    StoreError::SerdeError("Failed to serialize webhook data".to_string(), e)
                })
                .map(Some),
            None => Ok(None),
        }?;

        Ok(ConnectorRowNew {
            id: Uuid::now_v7(),
            tenant_id: self.tenant_id,
            alias: self.alias.clone(),
            connector_type: self.connector_type.clone().into(),
            provider: self.provider.clone().into(),
            sensitive,
            data,
        })
    }
}
