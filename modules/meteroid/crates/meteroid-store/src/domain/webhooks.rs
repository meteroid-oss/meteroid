use chrono::NaiveDateTime;
use diesel_models::webhooks::{WebhookInEventRow, WebhookInEventRowNew};
use o2o::o2o;
use uuid::Uuid;

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
    /// Provider-side event id (extracted from the parsed webhook). Drives the
    /// idempotency unique index on (provider_config_id, provider_event_id).
    pub provider_event_id: Option<String>,
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
    pub provider_event_id: Option<String>,
}
