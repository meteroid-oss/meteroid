use crate::api_rest::subscriptions::model::{
    CreateSubscriptionAddOn, CreateSubscriptionComponents,
};
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{
    CheckoutSessionId, CouponId, CustomerId, PlanVersionId, SubscriptionId, string_serde,
    string_serde_opt, string_serde_vec,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CheckoutSessionStatus {
    Created,
    AwaitingPayment,
    Completed,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CheckoutType {
    SelfServe,
    SubscriptionActivation,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CheckoutSession {
    #[serde(with = "string_serde")]
    pub id: CheckoutSessionId,
    #[serde(with = "string_serde")]
    pub customer_id: CustomerId,
    #[serde(with = "string_serde")]
    pub plan_version_id: PlanVersionId,
    pub coupon_code: Option<String>,
    pub billing_start_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i32>,
    pub net_terms: Option<i32>,
    pub trial_duration_days: Option<i32>,
    pub status: CheckoutSessionStatus,
    pub checkout_type: CheckoutType,
    pub created_at: DateTime<Utc>,
    /// When the session expires. None means the session never expires.
    pub expires_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, with = "string_serde_opt")]
    pub subscription_id: Option<SubscriptionId>,
    pub checkout_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateCheckoutSessionRequest {
    /// Customer ID or alias
    #[schema(format = "CustomerId or customer alias")]
    pub customer_id: String,
    #[serde(with = "string_serde")]
    pub plan_version_id: PlanVersionId,
    pub coupon_code: Option<String>,
    #[schema(value_type = Option<String>, format = "date")]
    pub billing_start_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i32>,
    pub net_terms: Option<i32>,
    pub trial_duration_days: Option<i32>,
    /// Session expiry time in hours. Default is 1 hour for self-serve checkout.
    pub expires_in_hours: Option<u32>,

    // Additional subscription parameters
    #[schema(value_type = Option<String>, format = "date")]
    pub end_date: Option<NaiveDate>,
    /// If false, invoices will stay in Draft until manually reviewed and finalized. Default is true.
    pub auto_advance_invoices: Option<bool>,
    /// Automatically try to charge the customer's configured payment method on finalize. Default is true.
    pub charge_automatically: Option<bool>,
    pub invoice_memo: Option<String>,
    #[schema(value_type = Option<String>, format = "decimal")]
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    pub purchase_order: Option<String>,

    // Complex parameters
    pub components: Option<CreateSubscriptionComponents>,
    pub add_ons: Option<Vec<CreateSubscriptionAddOn>>,

    // Multiple coupons (alternative to single coupon_code)
    #[serde(default, with = "string_serde_vec")]
    pub coupon_ids: Vec<CouponId>,

    // Custom metadata
    #[schema(value_type = Option<serde_json::Value>)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateCheckoutSessionResponse {
    pub session: CheckoutSession,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GetCheckoutSessionResponse {
    pub session: CheckoutSession,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListCheckoutSessionsResponse {
    pub sessions: Vec<CheckoutSession>,
}

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema, Validate)]
#[into_params(parameter_in = Query)]
pub struct ListCheckoutSessionsQuery {
    #[serde(default, with = "string_serde_opt")]
    pub customer_id: Option<CustomerId>,
    pub status: Option<CheckoutSessionStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CancelCheckoutSessionResponse {
    pub session: CheckoutSession,
}
