use crate::api_rest::model::PaginatedRequest;
use utoipa::ToSchema;
use validator::Validate;

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct SubscriptionRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    pub customer_id: Option<String>,
    pub plan_id: Option<String>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct Subscription {
    pub id: String,
    pub customer_id: String,
    pub customer_name: String,
    pub customer_alias: Option<String>,
    pub billing_day: i16,
    pub currency: String,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct SubscriptionDetails {
    pub id: String,
    pub customer_id: String,
    pub customer_name: String,
    pub customer_alias: Option<String>,
    pub billing_day: i16,
    pub currency: String,
}
