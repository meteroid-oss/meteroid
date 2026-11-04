use common_domain::ids::{EventId, WebhookDeliveryId, WebhookEndpointId, string_serde};
use meteroid_rest_common::datetime::ApiDateTime;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

use meteroid_rest_common::model::{PaginatedRequest, PaginationResponse};

// ── Enums ──────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WebhookEndpointDisabledReason {
    Manual,
    AutoFailures,
    Gone,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WebhookDeliveryStatus {
    Pending,
    InFlight,
    Succeeded,
    Failed,
    Cancelled,
}

// ── Responses ──────────────────────────────────────────────────

/// A custom header sent with every delivery. A sensitive header never returns its
/// value; `set` says whether it has one.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct WebhookHeader {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub sensitive: bool,
    pub set: bool,
}

/// A destination Meteroid POSTs signed event payloads to.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct WebhookEndpoint {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: WebhookEndpointId,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Subscribed event types. Empty means every event type except Beta ones, which are sent
    /// only to endpoints that list them.
    pub event_types: Vec<String>,
    pub disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<WebhookEndpointDisabledReason>,
    /// Failures since the last success, reset to 0 on any 2xx.
    pub consecutive_failures: i32,
    /// How many deliveries this endpoint may have in flight at once. Read-only; it is
    /// set from the tenant's environment when the endpoint is created.
    pub max_in_flight: i32,
    /// Custom headers sent with every delivery.
    pub headers: Vec<WebhookHeader>,
    /// A sensitive header still waits for its value; the endpoint cannot be enabled
    /// until it is set.
    pub needs_setup: bool,
    /// Deliveries started per second, at most.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_sec: Option<i32>,
    /// The endpoint was unreachable several times in a row: nothing is sent before
    /// this time, then it is retried one delivery at a time until it answers again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_until: Option<ApiDateTime>,
    pub created_at: ApiDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failure_at: Option<ApiDateTime>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct WebhookEndpointListResponse {
    pub data: Vec<WebhookEndpoint>,
}

/// The signing secret is returned in full here and never again outside the reveal
/// and rotate endpoints.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct CreatedWebhookEndpoint {
    #[serde(flatten)]
    pub endpoint: WebhookEndpoint,
    pub secret: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct WebhookEndpointSecret {
    pub secret: String,
}

/// One event queued for one endpoint, with the state of its retry cycle.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct WebhookDelivery {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: WebhookDeliveryId,
    #[serde(serialize_with = "string_serde::serialize")]
    pub message_id: EventId,
    #[serde(serialize_with = "string_serde::serialize")]
    pub endpoint_id: WebhookEndpointId,
    pub event_type: String,
    pub status: WebhookDeliveryStatus,
    pub attempt_count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_response_status: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// True when the delivery was created by a resend or a test event.
    pub manual: bool,
    pub created_at: ApiDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<ApiDateTime>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct WebhookDeliveryListResponse {
    pub data: Vec<WebhookDelivery>,
    pub pagination_meta: PaginationResponse,
}

// ── Requests ───────────────────────────────────────────────────

/// A custom header to send with every delivery. A sensitive header is write-only:
/// it is never returned, and on update sending it without a value keeps its value.
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct WebhookHeaderInput {
    pub name: String,
    pub value: Option<String>,
    #[serde(default)]
    pub sensitive: bool,
}

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct CreateWebhookEndpointRequest {
    /// HTTPS destination. Private and loopback addresses are rejected unless the
    /// instance is configured to allow them.
    #[validate(length(min = 1, max = 2048))]
    pub url: String,
    #[validate(length(max = 512))]
    pub description: Option<String>,
    /// Event types to subscribe to. Omit or leave empty to receive every event type except Beta
    /// ones, which are sent only to endpoints that list them.
    #[serde(default)]
    #[validate(length(max = 100))]
    pub event_types: Vec<String>,
    /// Custom headers sent with every delivery.
    #[serde(default)]
    pub headers: Vec<WebhookHeaderInput>,
    /// Deliveries started per second, at most (1 to 1000).
    pub rate_limit_per_sec: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateWebhookEndpointRequest {
    #[validate(length(min = 1, max = 2048))]
    pub url: Option<String>,
    /// Omit to leave unchanged; send `null` or an empty string to clear.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// Replaces the subscription list. An empty array subscribes to every event type except Beta
    /// ones, which are sent only to endpoints that list them.
    #[validate(length(max = 100))]
    pub event_types: Option<Vec<String>>,
    /// Re-enabling an endpoint also resets its consecutive failure count.
    pub disabled: Option<bool>,
    /// Replaces the custom header list. An empty array removes every header.
    pub headers: Option<Vec<WebhookHeaderInput>>,
    /// Omit to leave unchanged; send `null` to remove the rate limit.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<i32>)]
    pub rate_limit_per_sec: Option<Option<i32>>,
}

#[derive(Clone, Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeliveryListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    /// Only return deliveries in this state.
    pub status: Option<WebhookDeliveryStatus>,
}
