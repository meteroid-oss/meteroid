use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, SubscriptionId};
use serde::{Deserialize, Serialize, Serializer};
use serde_with::skip_serializing_none;
use std::fmt::Debug;
use std::str::FromStr;

pub struct CompanyId(pub String);
pub struct DealId(pub String);

#[skip_serializing_none]
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
    pub associations: Option<Vec<Association>>,
}

#[skip_serializing_none]
#[derive(Serialize)]
pub struct Association {
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

impl BatchUpsertResponse {
    pub fn get_company_id(&self, meteroid_id: CustomerId) -> Option<CompanyId> {
        self.results
            .iter()
            .find(|r| r.get_meteroid_customer_id() == Some(meteroid_id))
            .map(|r| CompanyId(r.id.clone()))
    }

    pub fn get_deal_id(&self, meteroid_id: SubscriptionId) -> Option<DealId> {
        self.results
            .iter()
            .find(|r| r.get_meteroid_subscription_id() == Some(meteroid_id))
            .map(|r| DealId(r.id.clone()))
    }
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

impl BatchUpsertItemResponse {
    pub fn get_meteroid_customer_id(&self) -> Option<CustomerId> {
        self.properties
            .get("meteroid_customer_id")
            .and_then(|v| v.as_str())
            .and_then(|s| CustomerId::from_str(s).ok())
    }

    pub fn get_meteroid_subscription_id(&self) -> Option<SubscriptionId> {
        self.properties
            .get("meteroid_subscription_id")
            .and_then(|v| v.as_str())
            .and_then(|s| SubscriptionId::from_str(s).ok())
    }
}

#[derive(Debug, Deserialize)]
pub struct StandardErrorResponse {
    pub status: String,
    pub category: String,
    pub message: String,
}
