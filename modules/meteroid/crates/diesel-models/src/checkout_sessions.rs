use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{
    CheckoutSessionId, CouponId, CustomerId, PlanVersionId, ResourceId, SubscriptionId, TenantId,
    TrialOfferId,
};
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use rust_decimal::Decimal;

use crate::enums::{CheckoutSessionStatusEnum, CheckoutTypeEnum};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::checkout_session)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CheckoutSessionRow {
    pub id: CheckoutSessionId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: Option<PlanVersionId>,

    // Basic subscription parameters
    pub billing_start_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i16>,
    pub net_terms: Option<i32>,
    pub trial_duration_days: Option<i32>,
    pub end_date: Option<NaiveDate>,

    // Billing options
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<Decimal>,
    pub purchase_order: Option<String>,

    pub components: Option<serde_json::Value>,
    pub add_ons: Option<serde_json::Value>,

    // Coupons
    pub coupon_code: Option<String>,
    pub coupon_ids: Vec<Option<CouponId>>,

    // Session state
    pub status: CheckoutSessionStatusEnum,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub subscription_id: Option<SubscriptionId>,
    pub metadata: Option<serde_json::Value>,
    pub checkout_type: CheckoutTypeEnum,
    // Payment configuration
    pub payment_methods_config: Option<serde_json::Value>,
    pub change_date: Option<NaiveDate>,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
    // What the coupons were before a payment attempt changed them, until it settles or fails.
    pub coupon_code_before_payment: Option<String>,
    pub coupon_ids_before_payment: Option<Vec<Option<CouponId>>>,
    /// The plan option bought, by alias; its currency is the customer's.
    pub plan_option: Option<String>,
    /// The stored resource the subscription it creates is for.
    pub resource_id: Option<ResourceId>,
    /// With `trial_duration_days`: whether that trial is free; `None`: as the plan's trial.
    pub trial_free: Option<bool>,
    /// The trial offer it gives, over `trial_duration_days`.
    pub trial_offer_id: Option<TrialOfferId>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::checkout_session)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CheckoutSessionRowNew {
    pub id: CheckoutSessionId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: Option<PlanVersionId>,

    // Basic subscription parameters
    pub billing_start_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i16>,
    pub net_terms: Option<i32>,
    pub trial_duration_days: Option<i32>,
    pub end_date: Option<NaiveDate>,

    // Billing options
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<Decimal>,
    pub purchase_order: Option<String>,

    // Payment configuration
    pub payment_methods_config: Option<serde_json::Value>,

    pub components: Option<serde_json::Value>,
    pub add_ons: Option<serde_json::Value>,

    // Coupons
    pub coupon_code: Option<String>,
    pub coupon_ids: Vec<Option<CouponId>>,

    // Session state
    pub expires_at: Option<DateTime<Utc>>,
    pub metadata: Option<serde_json::Value>,
    pub checkout_type: CheckoutTypeEnum,
    pub subscription_id: Option<SubscriptionId>,
    pub change_date: Option<NaiveDate>,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
    pub plan_option: Option<String>,
    pub resource_id: Option<ResourceId>,
    /// With `trial_duration_days`: whether that trial is free; `None`: as the plan's trial.
    pub trial_free: Option<bool>,
    /// The trial offer it gives, over `trial_duration_days`.
    pub trial_offer_id: Option<TrialOfferId>,
}
