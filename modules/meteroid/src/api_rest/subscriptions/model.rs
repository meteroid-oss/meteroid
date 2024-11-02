use crate::api_rest::model::PaginatedRequest;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct SubscriptionRequest {
    pub pagination: Option<PaginatedRequest>,
    pub customer_id: Option<Uuid>,
    pub plan_id: Option<Uuid>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct Subscription {
    pub id: Uuid,
}
