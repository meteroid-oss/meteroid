use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

#[derive(Serialize)]
pub struct BatchUpsertItemRequest {
    /// The ID of the object to update
    pub id: String,
    /// The name of a property whose values are unique for this object
    #[serde(rename = "idProperty")]
    pub id_property: Option<String>,
    /// In each input object, set this field to a unique ID value to enable more granular debugging for error responses.
    /// Learn more about [multi-status errors](https://developers.hubspot.com/docs/reference/api/other-resources/error-handling#multi-status-errors).
    #[serde(rename = "objectWriteTraceId")]
    pub object_write_trace_id: Option<String>,
    pub properties: serde_json::Value,
}

#[derive(Serialize)]
pub struct BatchCreateItemRequest {
    pub properties: serde_json::Value,
}

#[derive(Serialize)]
pub struct BatchActionRequest<T: Serialize> {
    pub inputs: Vec<T>,
}

pub type BatchCreateRequest = BatchActionRequest<BatchCreateItemRequest>;
pub type BatchUpsertRequest = BatchActionRequest<BatchUpsertItemRequest>;

#[derive(Debug, Deserialize)]
pub struct BatchUpsertResponse {
    #[serde(rename = "completedAt")]
    pub completed_at: DateTime<Utc>,
    #[serde(rename = "startedAt")]
    pub started_at: DateTime<Utc>,
    pub status: String,
    pub results: Vec<BatchUpsertItemResponse>,
    #[serde(rename = "numErrors")]
    pub num_errors: Option<i32>, // for status_207 status responses (multiple statuses)
    pub errors: Vec<StandardErrorResponse>, // for status_207 responses (multiple statuses)
}

#[derive(Debug, Deserialize)]
pub struct BatchUpsertItemResponse {
    pub id: String,
    pub new: bool,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: DateTime<Utc>,
    pub properties: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct StandardErrorResponse {
    pub status: String,
    pub category: String,
    pub message: String,
}
