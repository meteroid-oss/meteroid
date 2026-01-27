use crate::domain::enums::{ConnectorProviderEnum, ConnectorTypeEnum};
use crate::errors::{StoreError, StoreErrorReport};
use crate::{StoreResult, json_value_ser, json_value_serde};
use chrono::{DateTime, NaiveDateTime, Utc};
use common_domain::ids::{BaseId, ConnectorId, TenantId};
use diesel_models::connectors::{ConnectorRow, ConnectorRowNew};
use error_stack::ResultExt;
use meteroid_oauth::model::OAuthTokens;
use o2o::o2o;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::str::FromStr;

#[derive(Clone, Debug)]
pub struct Connector {
    pub id: ConnectorId,
    pub created_at: NaiveDateTime,
    pub tenant_id: TenantId,
    pub alias: String,
    pub connector_type: ConnectorTypeEnum,
    pub provider: ConnectorProviderEnum,
    pub data: Option<ProviderData>,
    // this gets turned into json string then is encrypted before storing
    pub sensitive: Option<ProviderSensitiveData>,
}

impl Connector {
    pub fn hubspot_data(&self) -> Option<&HubspotPublicData> {
        match &self.data {
            Some(ProviderData::Hubspot(data)) => Some(data),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ProviderData {
    Stripe(StripePublicData),
    Hubspot(HubspotPublicData),
    Pennylane(PennylanePublicData),
    Mock(MockPublicData),
}

json_value_ser!(ProviderData);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StripePublicData {
    pub api_publishable_key: String,
    pub account_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HubspotPublicData {
    pub auto_sync: bool,
    pub external_company_id: String, // hub_id
}

json_value_ser!(HubspotPublicData);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PennylanePublicData {
    pub external_company_id: String, // company.id
}

json_value_ser!(PennylanePublicData);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ProviderSensitiveData {
    Stripe(StripeSensitiveData),
    Hubspot(HubspotSensitiveData),
    Pennylane(PennylaneSensitiveData),
    Mock(MockSensitiveData),
}

impl ProviderSensitiveData {
    pub fn encrypt(&self, key: &SecretString) -> StoreResult<String> {
        let s = serde_json::to_string(self).map_err(|e| {
            StoreError::SerdeError(
                "Failed to serialize sensitive connector data".to_string(),
                e,
            )
        })?;

        crate::crypt::encrypt(key, s.as_str())
            .change_context(StoreError::CryptError("connector encryption error".into()))
    }

    pub fn decrypt(key: &SecretString, enc: &str) -> StoreResult<Self> {
        let decrypted = crate::crypt::decrypt(key, enc)
            .change_context(StoreError::CryptError("connector decryption error".into()))?;

        let sensitive: ProviderSensitiveData = serde_json::from_str(decrypted.expose_secret())
            .map_err(|e| {
                StoreError::SerdeError(
                    "Failed to deserialize sensitive connector data".to_string(),
                    e,
                )
            })?;
        Ok(sensitive)
    }
}

impl TryFrom<OAuthTokens> for PennylaneSensitiveData {
    type Error = StoreErrorReport;

    fn try_from(value: OAuthTokens) -> Result<Self, Self::Error> {
        Ok(PennylaneSensitiveData {
            access_token: value.access_token.expose_secret().to_string(),
            refresh_token: value
                .refresh_token
                .ok_or_else(|| StoreError::OauthError("missing refresh_token".into()))?
                .expose_secret()
                .to_string(),
            expires_at: value.expires_in.map(|duration| Utc::now() + duration),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StripeSensitiveData {
    pub api_secret_key: String,
    pub webhook_secret: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HubspotSensitiveData {
    pub refresh_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PennylaneSensitiveData {
    pub refresh_token: String,
    pub access_token: String,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MockPublicData {
    #[serde(default)]
    pub fail_payment_intent: bool,
    #[serde(default)]
    pub fail_setup_intent: bool,
}

json_value_ser!(MockPublicData);

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MockSensitiveData {}

impl Connector {
    pub fn from_row(key: &SecretString, row: ConnectorRow) -> StoreResult<Connector> {
        let sensitive = if let Some(s) = row.sensitive {
            let decrypted = ProviderSensitiveData::decrypt(key, &s)?;
            Some(decrypted)
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
    pub id: ConnectorId,
    pub created_at: NaiveDateTime,
    pub tenant_id: TenantId,
    pub alias: String,
    // pub data: Option<ProviderData>,
    #[map(~.into())]
    pub connector_type: ConnectorTypeEnum,
    #[map(~.into())]
    pub provider: ConnectorProviderEnum,
}

#[derive(Clone, Debug)]
pub struct ConnectorNew {
    pub tenant_id: TenantId,
    pub alias: String,
    pub connector_type: ConnectorTypeEnum,
    pub provider: ConnectorProviderEnum,
    pub data: Option<ProviderData>,
    pub sensitive: Option<ProviderSensitiveData>,
}

impl ConnectorNew {
    pub fn to_row(&self, key: &SecretString) -> StoreResult<ConnectorRowNew> {
        let sensitive = if let Some(sensitive) = &self.sensitive {
            let encrypted = sensitive.encrypt(key)?;
            Some(encrypted)
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
            id: ConnectorId::new(),
            tenant_id: self.tenant_id,
            alias: self.alias.clone(),
            connector_type: self.connector_type.clone().into(),
            provider: self.provider.clone().into(),
            sensitive,
            data,
        })
    }
}

#[skip_serializing_none]
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct ConnectionMeta {
    pub hubspot: Option<Vec<ConnectionMetaItem>>,
    pub pennylane: Option<Vec<ConnectionMetaItem>>,
}

impl ConnectionMeta {
    pub fn get_pennylane_id(&self, connector_id: ConnectorId) -> Option<i64> {
        self.pennylane
            .as_ref()
            .unwrap_or(&vec![])
            .iter()
            .find_map(|x| {
                if x.connector_id == connector_id {
                    i64::from_str(&x.external_id).ok()
                } else {
                    None
                }
            })
    }
}

json_value_serde!(ConnectionMeta);

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct ConnectionMetaItem {
    pub connector_id: ConnectorId,
    pub external_id: String,
    pub sync_at: DateTime<Utc>,
    pub external_company_id: String, // pennylane: company.id, hubspot: hub_id
}

json_value_serde!(ConnectionMetaItem);

#[derive(Clone, Debug)]
pub struct ConnectorAccessToken {
    pub connector_id: ConnectorId,
    pub external_company_id: String,
    pub access_token: SecretString,
    pub expires_at: Option<DateTime<Utc>>,
}
