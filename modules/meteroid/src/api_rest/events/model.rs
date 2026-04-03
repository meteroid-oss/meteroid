use common_domain::identifiers::{validator_code, validator_property_key};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use validator::Validate;

fn validate_event_code(code: &str) -> Result<(), validator::ValidationError> {
    validator_code(code)
}

fn validate_event_properties(
    props: &HashMap<String, String>,
) -> Result<(), validator::ValidationError> {
    for key in props.keys() {
        validator_property_key(key)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub struct Event {
    pub event_id: String,
    #[validate(custom(function = "validate_event_code"))]
    pub code: String,
    /// Either Meteroid's customer_id or an alias
    pub customer_id: String,
    pub timestamp: String,
    #[serde(default)]
    #[validate(custom(function = "validate_event_properties"))]
    pub properties: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
pub struct IngestEventsRequest {
    #[validate(length(min = 1, max = 100), nested)]
    pub events: Vec<Event>,
    /// allow ingesting events with timestamps more than a day in the past
    #[serde(default)]
    pub allow_backfilling: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IngestFailure {
    pub event_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IngestEventsResponse {
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub failures: Vec<IngestFailure>,
}
