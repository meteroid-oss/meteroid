use crate::api_rest::model::PaginatedRequest;
use utoipa::ToSchema;
use validator::Validate;

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct ProductFamilyFilters {
    pub search: Option<String>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct ProductFamilyListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    #[serde(flatten)]
    #[validate(nested)]
    pub plan_filters: ProductFamilyFilters,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct ProductFamily {
    pub id: String,
    pub name: String,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct ProductFamilyCreateRequest {
    pub name: String,
}
