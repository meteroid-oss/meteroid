use crate::domain::enums::WebhookOutEventTypeEnum;
use crate::domain::WebhookPage;
use crate::errors::StoreError;
use chrono::NaiveDateTime;
use diesel_models::webhooks::{WebhookInEventRow, WebhookInEventRowNew};
use error_stack::Report;
use o2o::o2o;
use secrecy::SecretString;
use serde::Serialize;
use url::Url;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct WebhookOutEndpoint {
    pub id: String,
    pub url: String,
    pub description: Option<String>,
    pub secret: SecretString,
    pub created_at: String,
    pub updated_at: String,
    pub events_to_listen: Vec<WebhookOutEventTypeEnum>,
    pub disabled: bool,
}

#[derive(Clone, Debug)]
pub struct WebhookOutEndpointListItem {
    pub id: String,
    pub url: String,
    pub description: Option<String>,
    pub events_to_listen: Vec<WebhookOutEventTypeEnum>,
    pub created_at: String,
    pub updated_at: String,
    pub disabled: bool,
}

impl TryFrom<svix::api::EndpointOut> for WebhookOutEndpointListItem {
    type Error = Report<StoreError>;

    fn try_from(value: svix::api::EndpointOut) -> Result<Self, Self::Error> {
        Ok(WebhookOutEndpointListItem {
            id: value.id,
            url: value.url,
            description: Some(value.description),
            created_at: value.created_at,
            updated_at: value.updated_at,
            events_to_listen: WebhookOutEventTypeEnum::from_svix_channels(&value.channels)?,
            disabled: value.disabled.unwrap_or(false),
        })
    }
}

impl TryFrom<svix::api::ListResponseEndpointOut> for WebhookPage<WebhookOutEndpointListItem> {
    type Error = Report<StoreError>;

    fn try_from(value: svix::api::ListResponseEndpointOut) -> Result<Self, Self::Error> {
        Ok(WebhookPage {
            data: value
                .data
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
            done: value.done,
            iterator: value.iterator,
            prev_iterator: value.prev_iterator,
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

pub enum WebhookOutMessageStatus {
    Success,
    Pending,
    Fail,
    Sending,
}

impl From<svix::api::MessageStatus> for WebhookOutMessageStatus {
    fn from(value: svix::api::MessageStatus) -> Self {
        match value {
            svix::api::MessageStatus::Success => WebhookOutMessageStatus::Success,
            svix::api::MessageStatus::Pending => WebhookOutMessageStatus::Pending,
            svix::api::MessageStatus::Fail => WebhookOutMessageStatus::Fail,
            svix::api::MessageStatus::Sending => WebhookOutMessageStatus::Sending,
        }
    }
}

impl From<WebhookOutMessageStatus> for svix::api::MessageStatus {
    fn from(value: WebhookOutMessageStatus) -> Self {
        match value {
            WebhookOutMessageStatus::Success => svix::api::MessageStatus::Success,
            WebhookOutMessageStatus::Pending => svix::api::MessageStatus::Pending,
            WebhookOutMessageStatus::Fail => svix::api::MessageStatus::Fail,
            WebhookOutMessageStatus::Sending => svix::api::MessageStatus::Sending,
        }
    }
}

pub enum WebhookOutStatusCodeClass {
    CodeNone,
    Code1xx,
    Code2xx,
    Code3xx,
    Code4xx,
    Code5xx,
}

impl From<WebhookOutStatusCodeClass> for svix::api::StatusCodeClass {
    fn from(value: WebhookOutStatusCodeClass) -> Self {
        match value {
            WebhookOutStatusCodeClass::CodeNone => svix::api::StatusCodeClass::CodeNone,
            WebhookOutStatusCodeClass::Code1xx => svix::api::StatusCodeClass::Code1xx,
            WebhookOutStatusCodeClass::Code2xx => svix::api::StatusCodeClass::Code2xx,
            WebhookOutStatusCodeClass::Code3xx => svix::api::StatusCodeClass::Code3xx,
            WebhookOutStatusCodeClass::Code4xx => svix::api::StatusCodeClass::Code4xx,
            WebhookOutStatusCodeClass::Code5xx => svix::api::StatusCodeClass::Code5xx,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
/// The message that is sent to the webhook
pub struct WebhookOutMessageNew {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: WebhookOutEventTypeEnum,
    pub payload: WebhookOutMessagePayload,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "object", rename_all = "snake_case")]
pub enum WebhookOutMessagePayload {
    Customer(serde_json::Value),
    Subscription(serde_json::Value),
}

impl TryFrom<WebhookOutMessageNew> for svix::api::MessageIn {
    type Error = Report<StoreError>;
    fn try_from(value: WebhookOutMessageNew) -> Result<Self, Self::Error> {
        let event_id = Some(value.id.clone());
        let event_type = value.event_type.to_string();
        let payload = serde_json::to_value(value).map_err(|e| {
            Report::from(StoreError::SerdeError(
                "Failed to serialize payload".to_string(),
                e,
            ))
        })?;

        let msg = svix::api::MessageIn {
            application: None,
            channels: None,
            event_id,
            event_type,
            payload,
            payload_retention_hours: None,
            payload_retention_period: None,
            tags: None,
            transformations_params: None,
        };

        Ok(msg)
    }
}

pub struct WebhookOutMessage {
    pub event_type: String,
    pub id: String,
    pub payload: serde_json::Value,
    pub timestamp: String,
}

impl From<svix::api::MessageOut> for WebhookOutMessage {
    fn from(value: svix::api::MessageOut) -> Self {
        WebhookOutMessage {
            event_type: value.event_type,
            id: value.id,
            payload: value.payload,
            timestamp: value.timestamp,
        }
    }
}

pub struct WebhookOutMessageAttempt {
    pub endpoint_id: String,
    pub id: String,
    pub msg: Option<Box<WebhookOutMessage>>,
    pub msg_id: String,
    pub response: String,
    // returns 0 in OSS version of svix
    pub response_duration_ms: i64,
    pub response_status_code: i32,
    pub timestamp: String,
    pub url: String,
}

impl From<svix::api::MessageAttemptOut> for WebhookOutMessageAttempt {
    fn from(value: svix::api::MessageAttemptOut) -> Self {
        WebhookOutMessageAttempt {
            endpoint_id: value.endpoint_id,
            id: value.id,
            msg: value.msg.map(|x| Box::new((*x).into())),
            msg_id: value.msg_id,
            response: value.response,
            response_duration_ms: value.response_duration_ms,
            response_status_code: value.response_status_code,
            timestamp: value.timestamp,
            url: value.url,
        }
    }
}

pub struct WebhookOutListMessageAttemptFilter {
    pub limit: Option<i32>,
    pub iterator: Option<String>,
    pub event_types: Option<Vec<String>>,
    pub status: Option<WebhookOutMessageStatus>,
    pub status_code_class: Option<WebhookOutStatusCodeClass>,
}

impl From<WebhookOutListMessageAttemptFilter> for svix::api::MessageAttemptListByEndpointOptions {
    fn from(value: WebhookOutListMessageAttemptFilter) -> Self {
        svix::api::MessageAttemptListByEndpointOptions {
            iterator: value.iterator,
            limit: value.limit,
            event_types: value.event_types,
            before: None,
            after: None,
            channel: None,
            tag: None,
            status: value.status.map(Into::into),
            status_code_class: value.status_code_class.map(Into::into),
            with_content: Some(true),
            with_msg: Some(true),
            endpoint_id: None,
        }
    }
}

impl From<svix::api::ListResponseMessageAttemptOut> for WebhookPage<WebhookOutMessageAttempt> {
    fn from(value: svix::api::ListResponseMessageAttemptOut) -> Self {
        WebhookPage {
            data: value.data.into_iter().map(Into::into).collect(),
            done: value.done,
            iterator: value.iterator,
            prev_iterator: value.prev_iterator,
        }
    }
}

pub struct WebhookOutListEndpointFilter {
    pub limit: Option<i32>,
    pub iterator: Option<String>,
}

impl From<WebhookOutListEndpointFilter> for svix::api::EndpointListOptions {
    fn from(value: WebhookOutListEndpointFilter) -> Self {
        svix::api::EndpointListOptions {
            iterator: value.iterator,
            limit: value.limit,
            order: None,
        }
    }
}

pub enum WebhookOutCreateMessageResult {
    Conflict,
    NotFound,
    SvixNotConfigured,
    Created(WebhookOutMessage),
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
