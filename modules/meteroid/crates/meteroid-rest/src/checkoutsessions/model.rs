use crate::subscriptions::model::{
    CreateSubscriptionAddOn, CreateSubscriptionComponents, PaymentMethodsConfig,
};
use chrono::NaiveDate;
use common_domain::ids::{
    CheckoutSessionId, CouponId, CustomerId, PlanVersionId, ResourceId, SubscriptionId,
    string_serde, string_serde_opt, string_serde_vec,
};
use meteroid_rest_common::datetime::ApiDateTime;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[schema(title = "CheckoutSessionStatus")]
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
    PlanChange,
    AddonPurchase,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CheckoutSession {
    #[serde(with = "string_serde")]
    pub id: CheckoutSessionId,
    #[serde(with = "string_serde")]
    pub customer_id: CustomerId,
    #[serde(with = "string_serde")]
    pub plan_version_id: PlanVersionId,
    /// The alias of the plan option bought.
    pub option: Option<String>,
    pub coupon_code: Option<String>,
    pub billing_start_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i32>,
    pub net_terms: Option<i32>,
    pub trial_duration_days: Option<i32>,
    pub status: CheckoutSessionStatus,
    pub checkout_type: CheckoutType,
    pub created_at: ApiDateTime,
    /// When the session expires. None means the session never expires.
    pub expires_at: Option<ApiDateTime>,
    pub completed_at: Option<ApiDateTime>,
    #[serde(default, with = "string_serde_opt")]
    pub subscription_id: Option<SubscriptionId>,
    pub checkout_url: Option<String>,
    pub payment_methods_config: Option<PaymentMethodsConfig>,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
    /// The stored resource the subscription is for; absent for the customer.
    #[serde(default, with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub resource_id: Option<ResourceId>,
    /// On creation, what its subscription would be accepted with: `plan_sold_for_mismatch` (the
    /// plan is sold for another target), `metric_not_attributed` (its resource would never bill
    /// some metrics it prices, listed in `metrics_not_attributed`).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
    /// With `metric_not_attributed`: the codes of the metrics it prices that are not attributed
    /// to its resource's type, so its subscription would bill none of their usage.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub metrics_not_attributed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateCheckoutSessionRequest {
    /// Customer ID or alias
    #[schema(format = "CustomerId or customer alias")]
    pub customer_id: String,
    #[serde(with = "string_serde")]
    pub plan_version_id: PlanVersionId,
    /// The plan option bought, by alias (e.g. `annual`, as the plan names it) or id; one the plan
    /// sells in the customer's currency, else refused with 400. Defaults to the plan's default
    /// option sold in that currency.
    pub option: Option<String>,
    /// Code of the coupon to apply, used only when `coupon_ids` is empty. It names the coupon with
    /// exactly that code, else the only live coupon equal to it ignoring the case of ASCII
    /// letters. An unknown or ambiguous code is refused with 400. If the code no longer names a
    /// usable coupon when the customer pays, the payment is refused and nothing is charged.
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

    /// Payment methods configuration. If not specified, inherits from the invoicing entity.
    pub payment_methods_config: Option<PaymentMethodsConfig>,

    /// Absolute http(s) URL the customer is sent to after a successful checkout.
    /// `checkout_session_id` is appended as a query parameter. Without it the customer stays on
    /// the hosted confirmation page.
    pub success_url: Option<String>,
    /// Absolute http(s) URL offered to the customer to leave the checkout without paying.
    pub cancel_url: Option<String>,
    /// One of the customer's stored resources the subscription is for; omit for the customer.
    #[serde(default, with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub resource_id: Option<ResourceId>,
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
    #[param(inline)]
    pub status: Option<CheckoutSessionStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CancelCheckoutSessionResponse {
    pub session: CheckoutSession,
}
