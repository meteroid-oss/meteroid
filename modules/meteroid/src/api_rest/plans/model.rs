use crate::api_rest::model::PaginatedRequest;
use chrono::NaiveDateTime;
use common_domain::ids::{PlanId, ProductFamilyId, string_serde, string_serde_opt};
use o2o::o2o;
use serde_enum_str::{Deserialize_enum_str, Serialize_enum_str};
use utoipa::ToSchema;
use validator::Validate;

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct PlanListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    #[serde(default, with = "string_serde_opt")]
    pub product_family_id: Option<ProductFamilyId>,
    #[serde(flatten)]
    pub plan_filters: PlanFilters,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct PlanFilters {
    pub search: Option<String>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct Plan {
    #[serde(with = "string_serde")]
    pub id: PlanId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
    pub product_family_name: String,
    #[serde(with = "string_serde")]
    pub product_family_id: ProductFamilyId,
    // #[from(~.map(| v | v.into()))]
    // pub active_version: Option<PlanVersionInfo>,
    // pub draft_version: Option<Uuid>,
    pub has_draft_version: bool,
    pub subscription_count: Option<i64>,
}

#[derive(o2o, Clone, ToSchema, Deserialize_enum_str, Serialize_enum_str, Debug, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::enums::PlanTypeEnum)]
pub enum PlanTypeEnum {
    Standard,
    Free,
    Custom,
}

#[derive(o2o, Clone, ToSchema, Deserialize_enum_str, Serialize_enum_str, Debug, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::enums::PlanStatusEnum)]
pub enum PlanStatusEnum {
    Draft,
    Active,
    Inactive,
    Archived,
}
