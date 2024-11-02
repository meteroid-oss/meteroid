use crate::api_rest::model::PaginatedRequest;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct SubscriptionRequest {
    #[serde(flatten)]
    pub pagination: PaginatedRequest,
    pub customer_id: Option<Uuid>,
    pub plan_id: Option<Uuid>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub customer_name: String,
    pub customer_alias: Option<String>,
}
