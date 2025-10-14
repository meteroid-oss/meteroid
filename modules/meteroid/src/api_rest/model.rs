use meteroid_store::domain;
use o2o::o2o;
use serde_with::{DisplayFromStr, serde_as};
use utoipa::ToSchema;
use validator::Validate;

#[serde_as]
#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate, Copy, Clone)]
pub struct PaginatedRequest {
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[validate(range(min = 0))]
    pub page: Option<u32>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[validate(range(min = 1, max = 100))]
    pub per_page: Option<u32>,
}

impl From<PaginatedRequest> for domain::PaginationRequest {
    fn from(val: PaginatedRequest) -> domain::PaginationRequest {
        domain::PaginationRequest {
            page: val.page.unwrap_or(0),
            per_page: val.per_page,
        }
    }
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct PaginationResponse {
    pub page: u32,
    pub per_page: u32,
    pub total_items: u64,
    pub total_pages: u32,
}

pub trait PaginationExt {
    #[allow(clippy::wrong_self_convention)]
    fn into_response(&self, total_pages: u32, total_items: u64) -> PaginationResponse;
}

impl PaginationExt for PaginatedRequest {
    fn into_response(&self, total_pages: u32, total_items: u64) -> PaginationResponse {
        PaginationResponse {
            page: self.page.unwrap_or(0),
            per_page: self.per_page.unwrap_or(10),
            total_items,
            total_pages,
        }
    }
}

#[derive(o2o, ToSchema, serde::Serialize, serde::Deserialize, Clone, Debug)]
#[map_owned(meteroid_store::domain::enums::BillingPeriodEnum)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BillingPeriodEnum {
    Monthly,
    Quarterly,
    Annual,
}
