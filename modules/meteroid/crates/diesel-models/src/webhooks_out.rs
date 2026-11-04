use chrono::NaiveDateTime;
use common_domain::ids::{
    EventId, TenantId, WebhookAttemptId, WebhookDeliveryId, WebhookEndpointId,
};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, QueryableByName, Selectable};

use crate::enums::{WebhookDeliveryStatusEnum, WebhookEndpointDisabledReasonEnum};

#[derive(Queryable, Selectable, Identifiable, Debug, Clone)]
#[diesel(table_name = crate::schema::webhook_out_endpoint)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct WebhookOutEndpointRow {
    pub id: WebhookEndpointId,
    pub tenant_id: TenantId,
    pub url: String,
    pub description: Option<String>,
    #[diesel(deserialize_as = crate::extend::non_null_array::NonNullArray<String>)]
    pub event_types: Vec<String>,
    pub secret: String,
    pub previous_secret: Option<String>,
    pub previous_secret_expires_at: Option<NaiveDateTime>,
    pub disabled: bool,
    pub disabled_reason: Option<WebhookEndpointDisabledReasonEnum>,
    pub disabled_at: Option<NaiveDateTime>,
    pub consecutive_failures: i32,
    pub last_success_at: Option<NaiveDateTime>,
    pub last_failure_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub max_in_flight: i32,
    pub headers: serde_json::Value,
    pub sensitive_header_values: Option<String>,
    pub failing_since: Option<NaiveDateTime>,
    pub unreachable_streak: i32,
    pub paused_until: Option<NaiveDateTime>,
    pub rate_limit_per_sec: Option<i32>,
    pub legacy_svix_headers: bool,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::webhook_out_endpoint)]
pub struct WebhookOutEndpointRowNew {
    pub id: WebhookEndpointId,
    pub tenant_id: TenantId,
    pub url: String,
    pub description: Option<String>,
    pub event_types: Vec<String>,
    pub secret: String,
    pub max_in_flight: i32,
    pub headers: serde_json::Value,
    pub sensitive_header_values: Option<String>,
    pub disabled: bool,
    pub disabled_reason: Option<WebhookEndpointDisabledReasonEnum>,
    pub disabled_at: Option<NaiveDateTime>,
    pub rate_limit_per_sec: Option<i32>,
    pub legacy_svix_headers: bool,
}

#[derive(AsChangeset, Debug, Default)]
#[diesel(table_name = crate::schema::webhook_out_endpoint)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct WebhookOutEndpointRowPatch {
    pub url: Option<String>,
    pub description: Option<Option<String>>,
    pub event_types: Option<Vec<String>>,
    pub disabled: Option<bool>,
    pub disabled_reason: Option<Option<WebhookEndpointDisabledReasonEnum>>,
    pub disabled_at: Option<Option<NaiveDateTime>>,
    pub consecutive_failures: Option<i32>,
    pub headers: Option<serde_json::Value>,
    pub sensitive_header_values: Option<Option<String>>,
    pub rate_limit_per_sec: Option<Option<i32>>,
    pub paused_until: Option<Option<NaiveDateTime>>,
    pub updated_at: Option<NaiveDateTime>,
}

#[derive(Queryable, Selectable, Identifiable, Debug, Clone)]
#[diesel(table_name = crate::schema::webhook_out_message)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct WebhookOutMessageRow {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::webhook_out_message)]
pub struct WebhookOutMessageRowNew {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub event_type: String,
    pub payload: serde_json::Value,
}

#[derive(Queryable, QueryableByName, Selectable, Identifiable, Debug, Clone)]
#[diesel(table_name = crate::schema::webhook_out_delivery)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct WebhookOutDeliveryRow {
    pub id: WebhookDeliveryId,
    pub tenant_id: TenantId,
    pub message_id: EventId,
    pub endpoint_id: WebhookEndpointId,
    pub status: WebhookDeliveryStatusEnum,
    pub attempt_count: i32,
    pub next_attempt_at: Option<NaiveDateTime>,
    pub lease_expires_at: Option<NaiveDateTime>,
    pub last_response_status: Option<i16>,
    pub last_error: Option<String>,
    pub manual: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub completed_at: Option<NaiveDateTime>,
    pub claimed_at: Option<NaiveDateTime>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::webhook_out_delivery)]
pub struct WebhookOutDeliveryRowNew {
    pub id: WebhookDeliveryId,
    pub tenant_id: TenantId,
    pub message_id: EventId,
    pub endpoint_id: WebhookEndpointId,
    pub status: WebhookDeliveryStatusEnum,
    pub next_attempt_at: Option<NaiveDateTime>,
    pub manual: bool,
}

#[derive(Queryable, Selectable, Identifiable, Debug, Clone)]
#[diesel(table_name = crate::schema::webhook_out_attempt)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct WebhookOutAttemptRow {
    pub id: WebhookAttemptId,
    pub delivery_id: WebhookDeliveryId,
    pub attempt_number: i32,
    pub attempted_at: NaiveDateTime,
    pub duration_ms: i32,
    pub response_status: Option<i16>,
    pub response_body: Option<String>,
    pub error: Option<String>,
    pub succeeded: bool,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::webhook_out_attempt)]
pub struct WebhookOutAttemptRowNew {
    pub id: WebhookAttemptId,
    pub delivery_id: WebhookDeliveryId,
    pub attempt_number: i32,
    pub duration_ms: i32,
    pub response_status: Option<i16>,
    pub response_body: Option<String>,
    pub error: Option<String>,
    pub succeeded: bool,
}

/// A delivery row joined with its message's `event_type`, for the tenant-facing list.
#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = crate::schema::webhook_out_delivery)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct WebhookOutDeliveryListRow {
    #[diesel(embed)]
    pub delivery: WebhookOutDeliveryRow,
    #[diesel(select_expression = crate::schema::webhook_out_message::event_type)]
    #[diesel(select_expression_type = crate::schema::webhook_out_message::event_type)]
    pub event_type: String,
}
