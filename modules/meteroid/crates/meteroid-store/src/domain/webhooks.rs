use crate::domain::enums::WebhookOutEventTypeEnum;
use crate::errors::StoreError;
use crate::utils::gen::webhook_security;
use crate::StoreResult;
use chrono::NaiveDateTime;
use diesel_models::webhooks::{
    WebhookInEventRow, WebhookInEventRowNew, WebhookOutEndpointRow, WebhookOutEndpointRowNew,
    WebhookOutEventRow, WebhookOutEventRowNew,
};
use error_stack::ResultExt;
use itertools::Itertools;
use o2o::o2o;
use secrecy::{ExposeSecret, SecretString};
use url::Url;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct WebhookOutEndpoint {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub url: Url,
    pub description: Option<String>,
    pub secret: SecretString,
    pub created_at: NaiveDateTime,
    pub events_to_listen: Vec<WebhookOutEventTypeEnum>,
    pub enabled: bool,
}

impl WebhookOutEndpoint {
    pub fn from_row(
        key: &SecretString,
        row: WebhookOutEndpointRow,
    ) -> StoreResult<WebhookOutEndpoint> {
        let dec_sec = crate::crypt::decrypt(key, row.secret.as_str())
            .change_context(StoreError::CryptError("secret decryption error".into()))?;

        let dec_url = Url::parse(row.url.as_str())
            .change_context(StoreError::InvalidArgument("invalid url value".into()))?;

        Ok(WebhookOutEndpoint {
            id: row.id,
            tenant_id: row.tenant_id,
            url: dec_url,
            description: row.description,
            secret: dec_sec,
            created_at: row.created_at,
            events_to_listen: row
                .events_to_listen
                .into_iter()
                .flatten()
                .map_into()
                .collect(),
            enabled: row.enabled,
        })
    }
}

#[derive(Clone, Debug)]
pub struct WebhookOutEndpointNew {
    pub tenant_id: Uuid,
    pub url: Url,
    pub description: Option<String>,
    pub events_to_listen: Vec<WebhookOutEventTypeEnum>,
    pub enabled: bool,
}

impl WebhookOutEndpointNew {
    pub fn to_row(&self, key: &SecretString) -> StoreResult<WebhookOutEndpointRowNew> {
        let enc_secret =
            crate::crypt::encrypt(key, webhook_security::gen().expose_secret().as_str())
                .change_context(StoreError::CryptError("secret decryption error".into()))?;

        Ok(WebhookOutEndpointRowNew {
            id: Uuid::now_v7(),
            tenant_id: self.tenant_id,
            url: self.url.to_string(),
            description: self.description.clone(),
            secret: enc_secret,
            events_to_listen: self
                .events_to_listen
                .clone()
                .into_iter()
                .map_into()
                .collect(),
            enabled: self.enabled,
        })
    }
}

#[derive(Clone, Debug, o2o)]
#[from_owned(WebhookOutEventRow)]
#[owned_into(WebhookOutEventRow)]
pub struct WebhookOutEvent {
    pub id: Uuid,
    pub endpoint_id: Uuid,
    pub created_at: NaiveDateTime,
    #[map(~.into())]
    pub event_type: WebhookOutEventTypeEnum,
    pub request_body: String,
    pub response_body: Option<String>,
    pub http_status_code: Option<i16>,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(WebhookOutEventRowNew)]
#[owned_into(WebhookOutEventRowNew)]
#[ghosts(id: {uuid::Uuid::now_v7()})]
pub struct WebhookOutEventNew {
    pub endpoint_id: Uuid,
    pub created_at: NaiveDateTime,
    #[map(~.into())]
    pub event_type: WebhookOutEventTypeEnum,
    pub request_body: String,
    pub response_body: Option<String>,
    pub http_status_code: Option<i16>,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(WebhookInEventRowNew)]
pub struct WebhookInEventNew {
    pub id: Uuid,
    pub received_at: NaiveDateTime,
    pub action: Option<String>,
    pub key: String,
    pub processed: bool,
    pub attempts: i32,
    pub error: Option<String>,
    pub provider_config_id: Uuid,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(WebhookInEventRow)]
pub struct WebhookInEvent {
    pub id: Uuid,
    pub received_at: NaiveDateTime,
    pub action: Option<String>,
    pub key: String,
    pub processed: bool,
    pub attempts: i32,
    pub error: Option<String>,
    pub provider_config_id: Uuid,
}
