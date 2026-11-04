use common_domain::ids::{CouponId, PlanId, string_serde_vec, string_serde_vec_opt};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_rest_common::model::{PaginatedRequest, PaginationResponse};
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
pub struct PercentageDiscount {
    pub percentage: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct FixedDiscount {
    pub currency: String,
    pub amount: String,
}

/// Promotional credit added to the customer's credit balance when the subscription the coupon is
/// redeemed on starts. It never discounts an invoice and applies before tax. A customer redeems
/// it at most once, so the coupon cannot be `reusable`. Its code is case-insensitive: it is stored
/// upper-case and redeemed whatever the case.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct CreditDiscount {
    /// ISO 4217 code; it must match the currency of the subscription the coupon is redeemed on.
    pub currency: String,
    /// Credit added per redemption, in major units (e.g. "25.00").
    pub amount: String,
    /// Days the credit stays spendable after redemption, 1 to 3660. Absent: it never expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_days: Option<u32>,
    /// CUSTOMER (default): the credit pays any of the customer's subscriptions. SUBSCRIPTION: the
    /// credit pays only the subscription the code is redeemed on, and ends with it.
    #[serde(default)]
    pub scope: CouponCreditScope,
}

/// Which subscriptions a promotional credit coupon's credit pays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CouponCreditScope {
    /// Any of the customer's subscriptions.
    #[default]
    Customer,
    /// Only the subscription the code is redeemed on.
    Subscription,
}

impl From<meteroid_store::domain::coupons::CouponCreditScope> for CouponCreditScope {
    fn from(value: meteroid_store::domain::coupons::CouponCreditScope) -> Self {
        match value {
            meteroid_store::domain::coupons::CouponCreditScope::Customer => Self::Customer,
            meteroid_store::domain::coupons::CouponCreditScope::Subscription => Self::Subscription,
        }
    }
}

impl From<CouponCreditScope> for meteroid_store::domain::coupons::CouponCreditScope {
    fn from(value: CouponCreditScope) -> Self {
        match value {
            CouponCreditScope::Customer => Self::Customer,
            CouponCreditScope::Subscription => Self::Subscription,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CouponDiscount {
    Percentage(PercentageDiscount),
    Fixed(FixedDiscount),
    Credit(CreditDiscount),
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[schema(title = "CouponFilter")]
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
    /// A promotional-credit (`CREDIT`) code is stored upper-case; other codes as written. A code
    /// is redeemed by the coupon with exactly that code, else by the only live coupon equal to it
    /// ignoring the case of ASCII letters; several such coupons and no exact match make it
    /// ambiguous.
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub discount: CouponDiscount,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redemption_limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurring_value: Option<i32>,
    pub reusable: bool,
    pub disabled: bool,
    pub created_at: ApiDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<ApiDateTime>,
    pub redemption_count: i32,
    #[serde(serialize_with = "common_domain::ids::string_serde_vec::serialize")]
    pub plan_ids: Vec<PlanId>,
}

// ── Requests ───────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct CreateCouponRequest {
    /// 1 to 64 ASCII letters, digits, hyphens or underscores. A promotional-credit (`CREDIT`) code
    /// is stored upper-case. Codes are matched as described on `Coupon.code`.
    #[validate(length(min = 1, max = 64), custom(function = "validate_coupon_code"))]
    pub code: String,
    pub description: Option<String>,
    pub discount: CouponDiscount,
    pub expires_at: Option<ApiDateTime>,
    pub redemption_limit: Option<i32>,
    pub recurring_value: Option<i32>,
    /// Whether one customer may redeem the coupon more than once. Must be false for a
    /// promotional-credit (`CREDIT`) coupon.
    #[serde(default)]
    pub reusable: bool,
    #[serde(default, with = "string_serde_vec")]
    pub plan_ids: Vec<PlanId>,
}

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateCouponRequest {
    pub description: Option<String>,
    pub discount: Option<CouponDiscount>,
    #[serde(default, with = "string_serde_vec_opt")]
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
    /// Sort order. Format: `column.direction`. Allowed columns: `code`, `created_at`, `expires_at`. Direction: `asc` or `desc`. Default: `created_at.desc`.
    pub order_by: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct CouponListResponse {
    pub data: Vec<Coupon>,
    pub pagination_meta: PaginationResponse,
}
