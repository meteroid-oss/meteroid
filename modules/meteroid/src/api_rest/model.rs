use meteroid_store::domain;
use o2o::o2o;
use serde_with::{DisplayFromStr, serde_as};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

/// Serde serializer for NaiveDateTime → RFC 3339 with UTC "Z" suffix.
/// Use via `#[serde(serialize_with = "super::model::serialize_datetime")]`
pub fn serialize_datetime<S>(dt: &chrono::NaiveDateTime, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use chrono::SecondsFormat;
    serializer.serialize_str(&dt.and_utc().to_rfc3339_opts(SecondsFormat::Millis, true))
}

pub fn serialize_datetime_opt<S>(
    dt: &Option<chrono::NaiveDateTime>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use chrono::SecondsFormat;
    match dt {
        Some(dt) => {
            serializer.serialize_str(&dt.and_utc().to_rfc3339_opts(SecondsFormat::Millis, true))
        }
        None => serializer.serialize_none(),
    }
}

#[serde_as]
#[derive(
    ToSchema, serde::Serialize, serde::Deserialize, Validate, Copy, Clone, Debug, IntoParams,
)]
#[into_params(parameter_in = Query)]
pub struct PaginatedRequest {
    /// Page number (0-indexed)
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[validate(range(min = 0))]
    #[param(minimum = 0)]
    pub page: Option<u32>,
    /// Number of items per page
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[validate(range(min = 1, max = 100))]
    #[param(minimum = 1, maximum = 100)]
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

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Debug, Clone)]
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
    Semiannual,
    Annual,
}
