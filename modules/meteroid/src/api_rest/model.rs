use meteroid_store::domain;
use o2o::o2o;
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

impl From<PaginatedRequest> for domain::PaginationRequest {
    fn from(val: PaginatedRequest) -> domain::PaginationRequest {
        domain::PaginationRequest {
            page: val.page.unwrap_or(0),
            per_page: val.per_page,
        }
    }
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: u64,
}

#[derive(o2o, ToSchema, serde::Serialize, serde::Deserialize, Clone, Debug)]
#[map_owned(meteroid_store::domain::enums::BillingPeriodEnum)]
pub enum BillingPeriod {
    #[serde(rename = "MONTHLY")]
    Monthly,
    #[serde(rename = "QUARTERLY")]
    Quarterly,
    #[serde(rename = "ANNUAL")]
    Annual,
}
