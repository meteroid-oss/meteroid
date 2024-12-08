use crate::api_rest::model::PaginatedRequest;
use chrono::NaiveDateTime;
use utoipa::ToSchema;

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct PlanListRequest {
    #[serde(flatten)]
    pub pagination: PaginatedRequest,
    pub product_family_id: Option<String>,
    #[serde(flatten)]
    pub plan_filters: PlanFilters,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct PlanFilters {
    pub search: Option<String>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct Plan {
    pub local_id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub plan_type: String, // as PlanTypeEnum
    pub status: String,    // as PlanStatusEnum,
    pub product_family_name: String,
    pub product_family_id: String,
    // #[from(~.map(| v | v.into()))]
    // pub active_version: Option<PlanVersionInfo>,
    // pub draft_version: Option<Uuid>,
    pub has_draft_version: bool,
    pub subscription_count: Option<i64>,
}
