use crate::api_rest::model::{PaginatedRequest, PaginationResponse};
use common_domain::ids::{ProductFamilyId, string_serde};
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
    pub filters: ProductFamilyFilters,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct ProductFamilyListResponse {
    pub data: Vec<ProductFamily>,
    pub pagination_meta: PaginationResponse,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct ProductFamily {
    #[serde(with = "string_serde")]
    pub id: ProductFamilyId,
    pub name: String,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct ProductFamilyCreateRequest {
    pub name: String,
}
