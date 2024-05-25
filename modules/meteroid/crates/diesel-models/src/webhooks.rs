use crate::enums::WebhookOutEventTypeEnum;
use chrono::NaiveDateTime;

use diesel::{Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Queryable, Identifiable, Debug, Selectable)]
#[diesel(table_name = crate::schema::webhook_in_event)]
#[diesel(check_for_backend(diesel::pg::Pg))]
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

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::webhook_in_event)]
#[diesel(check_for_backend(diesel::pg::Pg))]
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

#[derive(Queryable, Identifiable, Debug)]
#[diesel(table_name = crate::schema::webhook_out_endpoint)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct WebhookOutEndpoint {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub url: String,
    pub description: Option<String>,
    pub secret: String,
    pub created_at: NaiveDateTime,
    pub events_to_listen: Vec<WebhookOutEventTypeEnum>,
    pub enabled: bool,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::webhook_out_endpoint)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct WebhookOutEndpointNew {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub url: String,
    pub description: Option<String>,
    pub secret: String,
    pub events_to_listen: Vec<WebhookOutEventTypeEnum>,
    pub enabled: bool,
}

#[derive(Queryable, Identifiable, Debug, Selectable)]
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

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::webhook_out_event)]
pub struct WebhookOutEventNew {
    pub id: Uuid,
    pub endpoint_id: Uuid,
    pub created_at: NaiveDateTime,
    pub event_type: WebhookOutEventTypeEnum,
    pub request_body: String,
    pub response_body: Option<String>,
    pub http_status_code: Option<i16>,
    pub error_message: Option<String>,
}
