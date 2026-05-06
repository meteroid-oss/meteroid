use common_domain::identifiers::validator_code;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use validator::Validate;

fn validate_event_code(code: &str) -> Result<(), validator::ValidationError> {
    validator_code(code)
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub struct Event {
    /// Unique event identifier. Max 255 characters. A UUID or ULID is recommended.
    #[validate(length(min = 1, max = 255))]
    pub event_id: String,
    /// Billable metric code. Max 512 characters.
    #[validate(custom(function = "validate_event_code"))]
    pub code: String,
    /// Meteroid customer ID or external customer alias.
    pub customer_id: String,
    /// RFC 3339 timestamp. Defaults to ingestion time if omitted.
    /// Must be between 24 hours ago and 1 hour from now. Set `allow_backfilling` to remove the past limit.
    #[schema(example = "2026-01-15T10:30:00Z")]
    pub timestamp: String,
    /// Arbitrary string key-value pairs used by billable metrics for filtering and aggregation.
    #[schema(example = json!({"region": "us-east-1", "tier": "pro", "bytes": "1048576"}))]
    #[serde(default)]
    pub properties: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub struct IngestEventsRequest {
    /// 1–100 events per request.
    #[validate(length(min = 1, max = 100), nested)]
    pub events: Vec<Event>,
    /// Allow events with timestamps more than 1 day in the past. Defaults to `false`.
    #[serde(default)]
    pub allow_backfilling: Option<bool>,
    /// Accept the batch even if some events fail validation. Defaults to `false`.
    /// When `true`, valid events are ingested and failures are reported in the response body.
    /// When `false` (default), any invalid event rejects the entire batch.
    #[serde(default)]
    pub allow_partial_failures: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IngestFailure {
    pub event_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IngestEventsResponse {
    /// Events that failed to ingest. Omitted when no failures.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub failures: Vec<IngestFailure>,
}
