use crate::api_rest::model::PaginatedRequest;
use chrono::NaiveDateTime;
use meteroid_store::domain;
use serde_enum_str::{Deserialize_enum_str, Serialize_enum_str};
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
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
    pub product_family_name: String,
    pub product_family_id: String,
    // #[from(~.map(| v | v.into()))]
    // pub active_version: Option<PlanVersionInfo>,
    // pub draft_version: Option<Uuid>,
    pub has_draft_version: bool,
    pub subscription_count: Option<i64>,
}

#[derive(Clone, ToSchema, Deserialize_enum_str, Serialize_enum_str, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanTypeEnum {
    Standard,
    Free,
    Custom,
}
impl From<domain::enums::PlanTypeEnum> for PlanTypeEnum {
    fn from(value: domain::enums::PlanTypeEnum) -> Self {
        match value {
            domain::enums::PlanTypeEnum::Standard => PlanTypeEnum::Standard,
            domain::enums::PlanTypeEnum::Free => PlanTypeEnum::Free,
            domain::enums::PlanTypeEnum::Custom => PlanTypeEnum::Custom,
        }
    }
}

#[derive(Clone, ToSchema, Deserialize_enum_str, Serialize_enum_str, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatusEnum {
    Draft,
    Active,
    Inactive,
    Archived,
}
impl From<domain::enums::PlanStatusEnum> for PlanStatusEnum {
    fn from(value: domain::enums::PlanStatusEnum) -> Self {
        match value {
            domain::enums::PlanStatusEnum::Draft => PlanStatusEnum::Draft,
            domain::enums::PlanStatusEnum::Active => PlanStatusEnum::Active,
            domain::enums::PlanStatusEnum::Inactive => PlanStatusEnum::Inactive,
            domain::enums::PlanStatusEnum::Archived => PlanStatusEnum::Archived,
        }
    }
}
