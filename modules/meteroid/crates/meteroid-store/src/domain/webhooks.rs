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
