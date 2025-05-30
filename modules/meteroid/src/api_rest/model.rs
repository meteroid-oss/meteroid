use meteroid_store::domain;
use serde_with::{DisplayFromStr, serde_as};
use utoipa::ToSchema;
use validator::Validate;

#[serde_as]
#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct PaginatedRequest {
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[validate(range(min = 0))]
    pub page: Option<u32>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[validate(range(min = 1, max = 100))]
    pub per_page: Option<u32>,
}

impl Into<domain::PaginationRequest> for PaginatedRequest {
    fn into(self) -> domain::PaginationRequest {
        domain::PaginationRequest {
            page: self.page.unwrap_or(0),
            per_page: self.per_page,
        }
    }
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: u64,
}
