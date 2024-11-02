use crate::api_rest::model::PaginatedRequest;
use uuid::Uuid;

#[derive(serde::Deserialize)]
pub struct SubscriptionRequest {
    pub pagination: Option<PaginatedRequest>,
    pub customer_id: Option<Uuid>,
    pub plan_id: Option<Uuid>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Subscription {
    pub id: Uuid,
}
