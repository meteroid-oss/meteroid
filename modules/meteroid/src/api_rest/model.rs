use serde_with::{DisplayFromStr, serde_as};
use utoipa::ToSchema;
use validator::Validate;

#[serde_as]
#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct PaginatedRequest {
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[validate(range(min = 0))]
    pub offset: Option<u32>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[validate(range(min = 1, max = 100))]
    pub limit: Option<u32>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: u64,
    pub offset: u32,
}
