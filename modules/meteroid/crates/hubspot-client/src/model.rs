use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, Serializer};
use std::fmt::Debug;

pub struct CompanyId(pub String);
pub struct DealId(pub String);

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub associations: Option<Vec<Association>>,
}

#[derive(Serialize)]
pub struct Association {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Associate>,
    pub to: Associate,
    pub types: Vec<AssociationType>,
}

#[derive(Serialize)]
pub struct Associate {
    pub id: String,
}

#[derive(Serialize)]
pub struct AssociationType {
    #[serde(rename = "associationCategory")]
    pub association_category: AssociationCategory,
    #[serde(rename = "associationTypeId")]
    pub association_type_id: AssociationTypeId,
}

#[derive(Serialize)]
pub enum AssociationCategory {
    #[serde(rename = "HUBSPOT_DEFINED")]
    HubspotDefined,
}

#[derive(Copy, Clone)]
#[repr(u16)]
pub enum AssociationTypeId {
    DealToCompany = 5,
}

impl Serialize for AssociationTypeId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u16(*self as u16)
    }
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
pub type BatchAssociationRequest = BatchActionRequest<Association>;

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
    pub errors: Option<Vec<StandardErrorResponse>>, // for status_207 responses (multiple statuses)
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
