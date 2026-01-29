//! Test data builders for creating subscriptions and other entities.

use chrono::NaiveDate;
use common_domain::ids::{CouponId, CustomerId, PlanVersionId, SubscriptionId};
use meteroid_store::Services;
use meteroid_store::domain::{
    CreateSubscription, CreateSubscriptionCoupon, CreateSubscriptionCoupons,
    SubscriptionActivationCondition, SubscriptionNew,
};

use crate::data::ids::{CUST_UBER_ID, PLAN_VERSION_1_LEETCODE_ID, TENANT_ID, USER_ID};

/// Builder for creating test subscriptions with sensible defaults.
#[derive(Clone)]
pub struct SubscriptionBuilder {
    customer_id: CustomerId,
    plan_version_id: PlanVersionId,
    start_date: NaiveDate,
    activation_condition: SubscriptionActivationCondition,
    trial_duration: Option<u32>,
    charge_automatically: bool,
    coupon_ids: Vec<CouponId>,
}

impl Default for SubscriptionBuilder {
    fn default() -> Self {
        Self {
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            activation_condition: SubscriptionActivationCondition::OnStart,
            trial_duration: None,
            charge_automatically: false,
            coupon_ids: vec![],
        }
    }
}

impl SubscriptionBuilder {
    /// Create a new builder with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the customer ID.
    pub fn customer(mut self, id: CustomerId) -> Self {
        self.customer_id = id;
        self
    }

    /// Set the plan version ID.
    pub fn plan_version(mut self, id: PlanVersionId) -> Self {
        self.plan_version_id = id;
        self
    }

    /// Set the start date.
    pub fn start_date(mut self, date: NaiveDate) -> Self {
        self.start_date = date;
        self
    }

    /// Set the activation condition.
    pub fn activation(mut self, condition: SubscriptionActivationCondition) -> Self {
        self.activation_condition = condition;
        self
    }

    /// Set OnStart activation.
    pub fn on_start(self) -> Self {
        self.activation(SubscriptionActivationCondition::OnStart)
    }

    /// Set OnCheckout activation.
    pub fn on_checkout(self) -> Self {
        self.activation(SubscriptionActivationCondition::OnCheckout)
    }

    /// Set Manual activation.
    pub fn manual(self) -> Self {
        self.activation(SubscriptionActivationCondition::Manual)
    }

    /// Set trial duration in days.
    pub fn trial_days(mut self, days: u32) -> Self {
        self.trial_duration = Some(days);
        self
    }

    /// Set no trial (explicit).
    /// Uses Some(0) to explicitly disable the plan's default trial.
    /// A trial duration of 0 is filtered out as "no trial" during subscription creation.
    pub fn no_trial(mut self) -> Self {
        self.trial_duration = Some(0);
        self
    }

    /// Enable auto-charge.
    pub fn auto_charge(mut self) -> Self {
        self.charge_automatically = true;
        self
    }

    /// Disable auto-charge (explicit).
    pub fn no_auto_charge(mut self) -> Self {
        self.charge_automatically = false;
        self
    }

    /// Add a coupon to the subscription.
    pub fn coupon(mut self, coupon_id: CouponId) -> Self {
        self.coupon_ids.push(coupon_id);
        self
    }

    /// Add multiple coupons to the subscription.
    pub fn coupons(mut self, coupon_ids: Vec<CouponId>) -> Self {
        self.coupon_ids.extend(coupon_ids);
        self
    }

    /// Create the subscription using the provided services.
    pub async fn create(self, services: &Services) -> SubscriptionId {
        let coupons = if self.coupon_ids.is_empty() {
            None
        } else {
            Some(CreateSubscriptionCoupons {
                coupons: self
                    .coupon_ids
                    .into_iter()
                    .map(|coupon_id| CreateSubscriptionCoupon { coupon_id })
                    .collect(),
            })
        };

        services
            .insert_subscription(
                CreateSubscription {
                    subscription: SubscriptionNew {
                        customer_id: self.customer_id,
                        plan_version_id: self.plan_version_id,
                        created_by: USER_ID,
                        net_terms: None,
                        invoice_memo: None,
                        invoice_threshold: None,
                        start_date: self.start_date,
                        end_date: None,
                        billing_start_date: None,
                        activation_condition: self.activation_condition,
                        trial_duration: self.trial_duration,
                        billing_day_anchor: None,
                        payment_strategy: None,
                        auto_advance_invoices: true,
                        charge_automatically: self.charge_automatically,
                        purchase_order: None,
                        backdate_invoices: false,
                        skip_checkout_session: false,
                    },
                    price_components: None,
                    add_ons: None,
                    coupons,
                },
                TENANT_ID,
            )
            .await
            .expect("Failed to create subscription")
            .id
    }
}

/// Convenience function to start building a subscription.
pub fn subscription() -> SubscriptionBuilder {
    SubscriptionBuilder::new()
}
