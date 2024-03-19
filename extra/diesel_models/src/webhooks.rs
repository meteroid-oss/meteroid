

use chrono::NaiveDateTime;
use uuid::Uuid;
use chrono::DateTime;
use chrono::offset::Utc;
use diesel::{Identifiable, Queryable};
use diesel::sql_types::{Array, Nullable};
use crate::enums::WebhookOutEventTypeEnum;

#[derive(Queryable, Debug)]
#[diesel(table_name = crate::schema::webhook_in_event)]
pub struct WebhookInEvent {
    pub id: Uuid,
    pub received_at: DateTime<Utc>,
    pub action: Option<String>,
    pub key: String,
    pub processed: bool,
    pub attempts: i32,
    pub error: Option<String>,
    pub provider_config_id: Uuid,
}

#[derive(Queryable, Debug)]
#[diesel(table_name = crate::schema::webhook_out_endpoint)]
pub struct WebhookOutEndpoint {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub url: String,
    pub description: Option<String>,
    pub secret: String,
    pub created_at: NaiveDateTime,
    pub events_to_listen: Vec<Option<Array<Nullable<WebhookOutEventTypeEnum>>>>,
    pub enabled: bool,
}

#[derive(Queryable, Debug)]
#[diesel(table_name = crate::schema::webhook_out_event)]
pub struct WebhookOutEvent {
    pub id: Uuid,
    pub endpoint_id: Uuid,
    pub created_at: NaiveDateTime,
    pub event_type: WebhookOutEventTypeEnum,
    pub request_body: String,
    pub response_body: Option<String>,
    pub http_status_code: Option<i16>,
    pub error_message: Option<String>,
}