use crate::api_rest::model::PaginatedRequest;
use common_domain::ids::CustomerId;
use common_domain::ids::{string_serde, SubscriptionId};
use common_domain::ids::{string_serde_opt, PlanId};
use utoipa::ToSchema;
use validator::Validate;

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct SubscriptionRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    #[serde(with = "string_serde_opt")]
    pub customer_id: Option<CustomerId>,
    #[serde(with = "string_serde_opt")]
    pub plan_id: Option<PlanId>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct Subscription {
    #[serde(with = "string_serde")]
    pub id: SubscriptionId,
    #[serde(with = "string_serde")]
    pub customer_id: CustomerId,
    pub customer_name: String,
    pub customer_alias: Option<String>,
    pub billing_day_anchor: i16,
    pub currency: String,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct SubscriptionDetails {
    #[serde(with = "string_serde")]
    pub id: SubscriptionId,
    #[serde(with = "string_serde")]
    pub customer_id: CustomerId,
    pub customer_name: String,
    pub customer_alias: Option<String>,
    pub billing_day_anchor: i16,
    pub currency: String,
}
