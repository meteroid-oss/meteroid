use crate::api_rest::model::{PaginatedRequest, PaginationResponse};
use chrono::NaiveDateTime;
use common_domain::ids::{CouponId, PlanId};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

fn validate_coupon_code(code: &str) -> Result<(), validator::ValidationError> {
    if !code
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(validator::ValidationError::new("invalid_coupon_code"));
    }
    Ok(())
}

// ── Enums ──────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "discount_type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CouponDiscountRest {
    Percentage { percentage: String },
    Fixed { currency: String, amount: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CouponFilterEnum {
    All,
    Active,
    Inactive,
    Archived,
}

// ── Response ───────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Coupon {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: CouponId,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub discount: CouponDiscountRest,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::api_rest::model::serialize_datetime_opt"
    )]
    pub expires_at: Option<NaiveDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redemption_limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurring_value: Option<i32>,
    pub reusable: bool,
    pub disabled: bool,
    #[serde(serialize_with = "crate::api_rest::model::serialize_datetime")]
    pub created_at: NaiveDateTime,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::api_rest::model::serialize_datetime_opt"
    )]
    pub archived_at: Option<NaiveDateTime>,
    pub redemption_count: i32,
    #[serde(serialize_with = "common_domain::ids::string_serde_vec::serialize")]
    pub plan_ids: Vec<PlanId>,
}

// ── Requests ───────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct CreateCouponRequest {
    #[validate(length(min = 1, max = 64), custom(function = "validate_coupon_code"))]
    pub code: String,
    pub description: Option<String>,
    pub discount: CouponDiscountRest,
    pub expires_at: Option<NaiveDateTime>,
    pub redemption_limit: Option<i32>,
    pub recurring_value: Option<i32>,
    #[serde(default)]
    pub reusable: bool,
    #[serde(default)]
    pub plan_ids: Vec<PlanId>,
}

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateCouponRequest {
    pub description: Option<String>,
    pub discount: Option<CouponDiscountRest>,
    pub plan_ids: Option<Vec<PlanId>>,
}

// ── List ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CouponListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    pub search: Option<String>,
    #[param(inline)]
    pub filter: Option<CouponFilterEnum>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct CouponListResponse {
    pub data: Vec<Coupon>,
    pub pagination_meta: PaginationResponse,
}
