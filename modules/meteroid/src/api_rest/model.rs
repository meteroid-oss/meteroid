use serde_with::{serde_as, DisplayFromStr};
use utoipa::OpenApi;
use utoipa::ToSchema;

#[serde_as]
#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct PaginatedRequest {
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub offset: Option<u32>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub limit: Option<u32>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: u64,
    pub offset: u32,
}
