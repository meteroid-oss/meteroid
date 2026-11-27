use chrono::NaiveDateTime;
use common_domain::ids::{
    EventId, TenantId, WebhookAttemptId, WebhookDeliveryId, WebhookEndpointId,
};
use diesel_models::webhooks_out::{
    WebhookOutAttemptRow, WebhookOutDeliveryListRow, WebhookOutDeliveryRow, WebhookOutEndpointRow,
    WebhookOutMessageRow,
};
use o2o::o2o;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::domain::enums::TenantEnvironmentEnum;

/// Per-endpoint in-flight cap of a production endpoint. It bounds how much of the
/// delivery budget one endpoint can hold; higher means faster bursts, more reordering.
pub const MAX_IN_FLIGHT_PRODUCTION: i32 = 4;
pub const MAX_IN_FLIGHT_SANDBOX: i32 = 1;

/// Non-production tenants stay serialised: their bursts are cheap to delay and their
/// endpoints are the ones most likely to be dead.
pub fn max_in_flight_for(environment: TenantEnvironmentEnum) -> i32 {
    match environment {
        TenantEnvironmentEnum::Production => MAX_IN_FLIGHT_PRODUCTION,
        _ => MAX_IN_FLIGHT_SANDBOX,
    }
}

use crate::domain::enums::{WebhookDeliveryStatus, WebhookEndpointDisabledReason};

/// A tenant's outbound webhook destination. Signing secrets are never part of this type;
/// they are fetched explicitly through the reveal / delivery paths.
#[derive(Clone, Debug)]
pub struct WebhookEndpoint {
    pub id: WebhookEndpointId,
    pub tenant_id: TenantId,
    pub url: String,
    pub description: Option<String>,
    /// Empty means every event type.
    pub event_types: Vec<String>,
    pub disabled: bool,
    pub disabled_reason: Option<WebhookEndpointDisabledReason>,
    pub disabled_at: Option<NaiveDateTime>,
    pub consecutive_failures: i32,
    pub last_success_at: Option<NaiveDateTime>,
    pub last_failure_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    /// How many deliveries the worker may hold for this endpoint at once.
    pub max_in_flight: i32,
    /// Custom headers sent with every delivery. Sensitive values are never in here.
    pub headers: Vec<WebhookEndpointHeader>,
    /// Migrated from Svix: deliveries also carry the `svix-*` signature headers its
    /// receiver may still verify.
    pub legacy_svix_headers: bool,
    /// First failure since the last success: the start of the auto-disable window.
    pub failing_since: Option<NaiveDateTime>,
    /// Consecutive attempts that got no usable answer; see `PROBE_THRESHOLD`.
    pub unreachable_streak: i32,
    /// Nothing is sent before this; the endpoint is then probed one delivery at a time.
    pub paused_until: Option<NaiveDateTime>,
    /// Deliveries started per second, at most.
    pub rate_limit_per_sec: Option<i32>,
    /// The API version its payloads are rendered at, as `Meteroid-Version` names it; none for an
    /// endpoint created through v1, the dashboard or before versions were recorded (v1's
    /// payloads). Payloads are v1's whatever it says until v2 payloads ship (ADR-0013 §15).
    pub api_version: Option<String>,
}

impl WebhookEndpoint {
    pub fn matches_event_type(&self, event_type: &str) -> bool {
        self.event_types.is_empty() || self.event_types.iter().any(|t| t == event_type)
    }

    pub fn is_deliverable(&self) -> bool {
        !self.disabled && self.archived_at.is_none()
    }

    /// A sensitive header still waits for its value. Such an endpoint cannot be enabled
    /// until it is set.
    pub fn needs_setup(&self) -> bool {
        self.headers.iter().any(|h| h.sensitive && !h.set)
    }
}

/// One custom header as stored in the endpoint's `headers` column. A sensitive header
/// never carries its value here: it lives encrypted in `sensitive_header_values`, and
/// `set` says whether there is one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookEndpointHeader {
    pub name: String,
    pub value: Option<String>,
    pub sensitive: bool,
    pub set: bool,
}

/// A header as written through the API. A sensitive header given without a value keeps
/// the value it already had, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookHeaderInput {
    pub name: String,
    pub value: Option<String>,
    pub sensitive: bool,
}

/// Drops the encrypted secret columns: a `WebhookEndpoint` is safe to hand to any caller.
impl From<WebhookOutEndpointRow> for WebhookEndpoint {
    fn from(row: WebhookOutEndpointRow) -> Self {
        let headers = serde_json::from_value(row.headers).unwrap_or_else(|e| {
            log::error!("Unreadable headers on webhook endpoint {}: {e}", row.id);
            Vec::new()
        });

        WebhookEndpoint {
            id: row.id,
            tenant_id: row.tenant_id,
            url: row.url,
            description: row.description,
            event_types: row.event_types,
            disabled: row.disabled,
            disabled_reason: row.disabled_reason.map(Into::into),
            disabled_at: row.disabled_at,
            consecutive_failures: row.consecutive_failures,
            last_success_at: row.last_success_at,
            last_failure_at: row.last_failure_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
            archived_at: row.archived_at,
            max_in_flight: row.max_in_flight,
            headers,
            legacy_svix_headers: row.legacy_svix_headers,
            failing_since: row.failing_since,
            unreachable_streak: row.unreachable_streak,
            paused_until: row.paused_until,
            rate_limit_per_sec: row.rate_limit_per_sec,
            api_version: row.api_version,
        }
    }
}

#[derive(Clone, Debug)]
pub struct WebhookEndpointNew {
    pub url: String,
    pub description: Option<String>,
    pub event_types: Vec<String>,
    /// Resolved by the caller from the owning tenant's environment; never patched later.
    pub max_in_flight: i32,
    pub headers: Vec<WebhookHeaderInput>,
    pub rate_limit_per_sec: Option<i32>,
    /// The `Meteroid-Version` of the v2 request creating it; none through v1 or the dashboard.
    pub api_version: Option<String>,
}

/// An endpoint brought over from a pre-2.0 Svix server, with the signing secret its
/// receiver already verifies. It keeps sending the `svix-*` headers.
#[derive(Clone, Debug)]
pub struct ImportedSvixWebhookEndpoint {
    pub url: String,
    pub description: Option<String>,
    pub event_types: Vec<String>,
    pub disabled: bool,
    pub secret: SecretString,
    pub max_in_flight: i32,
    pub headers: Vec<WebhookHeaderInput>,
    pub rate_limit_per_sec: Option<i32>,
}

#[derive(Clone, Debug)]
pub enum SvixWebhookEndpointImport {
    Created(Box<WebhookEndpoint>),
    /// The tenant already has a non-archived endpoint with this url; nothing was written.
    AlreadyPresent(WebhookEndpointId),
}

#[derive(Clone, Debug, Default)]
pub struct WebhookEndpointPatch {
    pub id: WebhookEndpointId,
    pub url: Option<String>,
    /// `Some(None)` clears the description.
    pub description: Option<Option<String>>,
    pub event_types: Option<Vec<String>>,
    pub disabled: Option<bool>,
    /// Replaces the whole header list.
    pub headers: Option<Vec<WebhookHeaderInput>>,
    /// `Some(None)` removes the rate limit.
    pub rate_limit_per_sec: Option<Option<i32>>,
    /// Pins its payloads to another API version.
    pub api_version: Option<String>,
}

/// Decrypted signing material for one endpoint, used only by the delivery worker.
/// `previous_secrets` holds every secret rotated out within the grace window, the newest first.
/// `headers` holds every custom header that has a value, sensitive ones decrypted.
#[derive(Clone, Debug)]
pub struct WebhookEndpointWithSecrets {
    pub endpoint: WebhookEndpoint,
    pub secret: SecretString,
    pub previous_secrets: Vec<SecretString>,
    pub headers: Vec<(String, SecretString)>,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(WebhookOutMessageRow)]
pub struct WebhookOutMessage {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: NaiveDateTime,
}

#[derive(Clone, Debug)]
pub struct WebhookOutMessageNew {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub event_type: String,
    /// v1's payload, which a delivery without an API version sends.
    pub payload: serde_json::Value,
}

/// The payload of a message rendered at an API version with payloads of its own (ADR-0013 §15),
/// sent by the deliveries at that version.
#[derive(Clone, Debug, PartialEq)]
pub struct WebhookOutMessageVersion {
    pub api_version: String,
    pub payload: serde_json::Value,
}

/// An endpoint a message is delivered to, and the version of the payload it is sent: None for
/// v1's, else one of the message's [`WebhookOutMessageVersion`]s.
#[derive(Clone, Debug, PartialEq)]
pub struct WebhookOutTarget {
    pub endpoint_id: WebhookEndpointId,
    pub api_version: Option<String>,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(WebhookOutDeliveryRow)]
pub struct WebhookDelivery {
    pub id: WebhookDeliveryId,
    pub tenant_id: TenantId,
    pub message_id: EventId,
    pub endpoint_id: WebhookEndpointId,
    #[map(~.into())]
    pub status: WebhookDeliveryStatus,
    /// Retry budget spent in the current cycle: incremented by every claim, so a claim
    /// spends budget even when its outcome is never recorded, refunded by a probe of a
    /// paused endpoint and reset by a resend. Not the number of attempts made.
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
    /// The API version its payload was rendered at, fixed when it was created: None for v1's.
    pub api_version: Option<String>,
}

#[derive(Clone, Debug)]
pub struct WebhookDeliveryListItem {
    pub delivery: WebhookDelivery,
    pub event_type: String,
    /// Attempts made so far, across resends: see `WebhookDelivery::attempt_count` for
    /// the retry budget instead.
    pub attempts: i64,
}

impl From<WebhookOutDeliveryListRow> for WebhookDeliveryListItem {
    fn from(row: WebhookOutDeliveryListRow) -> Self {
        WebhookDeliveryListItem {
            delivery: row.delivery.into(),
            event_type: row.event_type,
            attempts: 0,
        }
    }
}

#[derive(Clone, Debug, o2o)]
#[from_owned(WebhookOutAttemptRow)]
pub struct WebhookAttempt {
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

#[derive(Clone, Debug)]
pub struct WebhookDeliveryDetails {
    pub delivery: WebhookDelivery,
    /// The message with the payload the delivery sends (at its `api_version`).
    pub message: WebhookOutMessage,
    pub attempts: Vec<WebhookAttempt>,
}

/// Everything one finished HTTP attempt writes back, applied in a single transaction.
/// `attempt_number` and `lease_expires_at` are the claim it was made under: the write
/// is dropped once the row has been reclaimed, resent or claimed again.
#[derive(Clone, Debug)]
pub struct WebhookAttemptOutcome {
    pub delivery_id: WebhookDeliveryId,
    pub endpoint_id: WebhookEndpointId,
    pub attempt_number: i32,
    pub lease_expires_at: NaiveDateTime,
    pub duration_ms: i32,
    pub response_status: Option<i16>,
    pub response_body: Option<String>,
    pub error: Option<String>,
    pub succeeded: bool,
    pub next_attempt_at: Option<NaiveDateTime>,
    pub final_status: WebhookDeliveryStatus,
    pub disable_endpoint: Option<WebhookEndpointDisabledReason>,
    /// No usable answer from the endpoint as a whole (see `is_endpoint_wide_failure`).
    pub endpoint_wide_failure: bool,
    /// Pause the endpoint until then.
    pub pause_endpoint_until: Option<NaiveDateTime>,
    /// This attempt was a probe of a paused endpoint: it does not spend retry budget.
    pub refund_attempt: bool,
}

/// Deliveries of one day by outcome. `retrying` ones have failed and are still retried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WebhookDailyStats {
    pub day: chrono::NaiveDate,
    pub succeeded: i64,
    pub failed: i64,
    pub retrying: i64,
    pub cancelled: i64,
}

/// Deliveries of one endpoint over a window by outcome, as [`WebhookDailyStats`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WebhookEndpointStats {
    pub endpoint_id: WebhookEndpointId,
    pub succeeded: i64,
    pub failed: i64,
    pub retrying: i64,
    pub cancelled: i64,
}
