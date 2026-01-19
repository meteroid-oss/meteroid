use chrono::{DateTime, Duration, NaiveDate, Utc};
use common_domain::ids::{
    BaseId, CheckoutSessionId, CouponId, CustomerId, PlanVersionId, SubscriptionId, TenantId,
};
use diesel_models::checkout_sessions::{CheckoutSessionRow, CheckoutSessionRowNew};
use diesel_models::enums::{CheckoutSessionStatusEnum, CheckoutTypeEnum};
use o2o::o2o;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::enums::SubscriptionPaymentStrategy;
use crate::domain::subscription_add_ons::CreateSubscriptionAddOns;
use crate::domain::subscription_components::CreateSubscriptionComponents;
use crate::domain::subscriptions::{CreateSubscription, SubscriptionNew};
use crate::domain::{
    CreateSubscriptionCoupon, CreateSubscriptionCoupons, SubscriptionActivationCondition,
};

#[derive(Debug, Clone, PartialEq, Eq, o2o)]
#[map_owned(CheckoutSessionStatusEnum)]
pub enum CheckoutSessionStatus {
    Created,
    AwaitingPayment,
    Completed,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, o2o)]
#[map_owned(CheckoutTypeEnum)]
pub enum CheckoutType {
    #[default]
    SelfServe,
    SubscriptionActivation,
}

/// Serializable payment strategy for JSONB storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CheckoutPaymentStrategy {
    Auto,
    Bank,
    External,
}

impl From<CheckoutPaymentStrategy> for SubscriptionPaymentStrategy {
    fn from(s: CheckoutPaymentStrategy) -> Self {
        match s {
            CheckoutPaymentStrategy::Auto => SubscriptionPaymentStrategy::Auto,
            CheckoutPaymentStrategy::Bank => SubscriptionPaymentStrategy::Bank,
            CheckoutPaymentStrategy::External => SubscriptionPaymentStrategy::External,
        }
    }
}

impl From<SubscriptionPaymentStrategy> for CheckoutPaymentStrategy {
    fn from(s: SubscriptionPaymentStrategy) -> Self {
        match s {
            SubscriptionPaymentStrategy::Auto => CheckoutPaymentStrategy::Auto,
            SubscriptionPaymentStrategy::Bank => CheckoutPaymentStrategy::Bank,
            SubscriptionPaymentStrategy::External => CheckoutPaymentStrategy::External,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CheckoutSession {
    pub id: CheckoutSessionId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: PlanVersionId,
    pub created_by: Uuid,

    // Basic subscription parameters
    pub billing_start_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i16>,
    pub net_terms: Option<i32>,
    pub trial_duration_days: Option<i32>,
    pub end_date: Option<NaiveDate>,

    // Billing options
    pub activation_condition: SubscriptionActivationCondition,
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<Decimal>,
    pub purchase_order: Option<String>,

    // Complex parameters
    pub payment_strategy: Option<CheckoutPaymentStrategy>,
    pub components: Option<CreateSubscriptionComponents>,
    pub add_ons: Option<CreateSubscriptionAddOns>,

    // Coupons
    pub coupon_code: Option<String>,
    pub coupon_ids: Vec<CouponId>,

    // Session state
    pub status: CheckoutSessionStatus,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub subscription_id: Option<SubscriptionId>,
    pub metadata: Option<serde_json::Value>,
    pub checkout_type: CheckoutType,
}

impl CheckoutSession {
    pub fn is_expired(&self) -> bool {
        self.status == CheckoutSessionStatus::Expired
            || self.expires_at.is_some_and(|exp| Utc::now() > exp)
    }

    pub fn is_completed(&self) -> bool {
        self.status == CheckoutSessionStatus::Completed
    }

    pub fn can_complete(&self) -> bool {
        // Allow completion from Created (first attempt) or AwaitingPayment (retry after failed payment)
        (self.status == CheckoutSessionStatus::Created
            || self.status == CheckoutSessionStatus::AwaitingPayment)
            && !self.is_expired()
    }

    pub fn to_subscription_new(&self, start_date: NaiveDate) -> SubscriptionNew {
        SubscriptionNew {
            customer_id: self.customer_id,
            plan_version_id: self.plan_version_id,
            created_by: self.created_by,
            net_terms: self.net_terms.map(|n| n as u32),
            invoice_memo: self.invoice_memo.clone(),
            invoice_threshold: self.invoice_threshold,
            start_date,
            end_date: self.end_date,
            billing_start_date: self.billing_start_date,
            activation_condition: self.activation_condition.clone(),
            trial_duration: self.trial_duration_days.map(|d| d as u32),
            billing_day_anchor: self.billing_day_anchor.map(|a| a as u16),
            payment_strategy: self.payment_strategy.clone().map(Into::into),
            auto_advance_invoices: self.auto_advance_invoices,
            charge_automatically: self.charge_automatically,
            purchase_order: self.purchase_order.clone(),
            backdate_invoices: false,
        }
    }

    pub fn to_create_subscription(
        &self,
        start_date: NaiveDate,
        coupon_ids: Vec<CouponId>,
    ) -> CreateSubscription {
        let coupons = if coupon_ids.is_empty() {
            None
        } else {
            Some(CreateSubscriptionCoupons {
                coupons: coupon_ids
                    .into_iter()
                    .map(|coupon_id| CreateSubscriptionCoupon { coupon_id })
                    .collect(),
            })
        };

        CreateSubscription {
            subscription: self.to_subscription_new(start_date),
            price_components: self.components.clone(),
            add_ons: self.add_ons.clone(),
            coupons,
        }
    }
}

impl From<CheckoutSessionRow> for CheckoutSession {
    fn from(row: CheckoutSessionRow) -> Self {
        let session_id = row.id;

        // TODO we could mark the subscription as errored
        let payment_strategy: Option<CheckoutPaymentStrategy> =
            row.payment_strategy.and_then(|v| {
                serde_json::from_value(v.clone()).map_err(|e| {
                    log::error!(
                        "Failed to deserialize payment_strategy for checkout session {}: {}. Raw value: {:?}",
                        session_id, e, v
                    );
                    e
                }).ok()
            });

        let components: Option<CreateSubscriptionComponents> = row.components.and_then(|v| {
            serde_json::from_value(v.clone()).map_err(|e| {
                log::error!(
                    "Failed to deserialize components for checkout session {}: {}. This may result in a subscription without the expected price components!",
                    session_id, e
                );
                e
            }).ok()
        });

        let add_ons: Option<CreateSubscriptionAddOns> = row.add_ons.and_then(|v| {
            serde_json::from_value(v.clone()).map_err(|e| {
                log::error!(
                    "Failed to deserialize add_ons for checkout session {}: {}. This may result in a subscription without the expected add-ons!",
                    session_id, e
                );
                e
            }).ok()
        });

        Self {
            id: row.id,
            tenant_id: row.tenant_id,
            customer_id: row.customer_id,
            plan_version_id: row.plan_version_id,
            created_by: row.created_by,
            billing_start_date: row.billing_start_date,
            billing_day_anchor: row.billing_day_anchor,
            net_terms: row.net_terms,
            trial_duration_days: row.trial_duration_days,
            end_date: row.end_date,
            activation_condition: row.activation_condition.into(),
            auto_advance_invoices: row.auto_advance_invoices,
            charge_automatically: row.charge_automatically,
            invoice_memo: row.invoice_memo,
            invoice_threshold: row.invoice_threshold,
            purchase_order: row.purchase_order,
            payment_strategy,
            components,
            add_ons,
            coupon_code: row.coupon_code,
            coupon_ids: row.coupon_ids.into_iter().flatten().collect(),
            status: row.status.into(),
            created_at: row.created_at,
            expires_at: row.expires_at,
            completed_at: row.completed_at,
            subscription_id: row.subscription_id,
            metadata: row.metadata,
            checkout_type: row.checkout_type.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreateCheckoutSession {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: PlanVersionId,
    pub created_by: Uuid,

    // Basic subscription parameters
    pub billing_start_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i16>,
    pub net_terms: Option<i32>,
    pub trial_duration_days: Option<i32>,
    pub end_date: Option<NaiveDate>,

    // Billing options
    pub activation_condition: SubscriptionActivationCondition,
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<Decimal>,
    pub purchase_order: Option<String>,

    // Complex parameters
    pub payment_strategy: Option<CheckoutPaymentStrategy>,
    pub components: Option<CreateSubscriptionComponents>,
    pub add_ons: Option<CreateSubscriptionAddOns>,

    // Coupons
    pub coupon_code: Option<String>,
    pub coupon_ids: Vec<CouponId>,

    // Session options
    pub expires_in_hours: Option<u32>,
    pub metadata: Option<serde_json::Value>,
    pub checkout_type: CheckoutType,
    pub subscription_id: Option<SubscriptionId>,
}

impl CreateCheckoutSession {
    pub fn into_row(self) -> CheckoutSessionRowNew {
        let expires_at = self
            .expires_in_hours
            .map(|h| Utc::now() + Duration::hours(h as i64));

        let payment_strategy = self
            .payment_strategy
            .and_then(|s| serde_json::to_value(s).ok());
        let components = self.components.and_then(|c| serde_json::to_value(c).ok());
        let add_ons = self.add_ons.and_then(|a| serde_json::to_value(a).ok());

        CheckoutSessionRowNew {
            id: CheckoutSessionId::new(),
            tenant_id: self.tenant_id,
            customer_id: self.customer_id,
            plan_version_id: self.plan_version_id,
            created_by: self.created_by,
            billing_start_date: self.billing_start_date,
            billing_day_anchor: self.billing_day_anchor,
            net_terms: self.net_terms,
            trial_duration_days: self.trial_duration_days,
            end_date: self.end_date,
            activation_condition: self.activation_condition.into(),
            auto_advance_invoices: self.auto_advance_invoices,
            charge_automatically: self.charge_automatically,
            invoice_memo: self.invoice_memo,
            invoice_threshold: self.invoice_threshold,
            purchase_order: self.purchase_order,
            payment_strategy,
            components,
            add_ons,
            coupon_code: self.coupon_code,
            coupon_ids: self.coupon_ids.into_iter().map(Some).collect(),
            expires_at,
            metadata: self.metadata,
            checkout_type: self.checkout_type.into(),
            subscription_id: self.subscription_id,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CheckoutCompletionResult {
    /// Checkout completed successfully - subscription and optionally invoice created
    Completed {
        subscription_id: SubscriptionId,
        transaction: Option<crate::domain::PaymentTransaction>,
    },
    /// Payment is pending (async payment method like SEPA) - awaiting webhook confirmation
    /// No subscription or invoice created yet - will be created when payment settles
    AwaitingPayment {
        transaction: crate::domain::PaymentTransaction,
    },
}
