//! Coupon test helpers.

use common_domain::ids::{CouponId, PlanId};
use meteroid_store::domain::coupons::{
    CouponDiscount, CouponNew, CouponPatch, CouponStatusPatch, CouponTerms,
};
use meteroid_store::repositories::coupons::CouponInterface;
use rust_decimal::Decimal;

use crate::data::ids::TENANT_ID;

use super::TestEnv;

impl TestEnv {
    /// Create a fixed amount coupon.
    ///
    /// # Arguments
    /// * `code` - Coupon code
    /// * `amount` - Discount amount in main currency unit (e.g., 10 for €10)
    /// * `currency` - Currency code (e.g., "EUR")
    pub async fn create_fixed_coupon(&self, code: &str, amount: i64, currency: &str) -> CouponId {
        self.create_coupon(CouponNew {
            code: code.to_string(),
            description: format!("Fixed {} {} discount", amount, currency),
            tenant_id: TENANT_ID,
            discount: CouponDiscount::Fixed {
                amount: Decimal::from(amount),
                currency: currency.to_string(),
            },
            expires_at: None,
            redemption_limit: None,
            recurring_value: None,
            reusable: false,
            plan_ids: vec![],
        })
        .await
    }

    /// Create a percentage coupon.
    ///
    /// # Arguments
    /// * `code` - Coupon code
    /// * `percentage` - Discount percentage (e.g., 10 for 10%)
    pub async fn create_percentage_coupon(&self, code: &str, percentage: u32) -> CouponId {
        self.create_coupon(CouponNew {
            code: code.to_string(),
            description: format!("{}% discount", percentage),
            tenant_id: TENANT_ID,
            discount: CouponDiscount::Percentage(Decimal::from(percentage)),
            expires_at: None,
            redemption_limit: None,
            recurring_value: None,
            reusable: false,
            plan_ids: vec![],
        })
        .await
    }

    /// Create a percentage coupon limited to specific billing cycles.
    ///
    /// # Arguments
    /// * `code` - Coupon code
    /// * `percentage` - Discount percentage (e.g., 10 for 10%)
    /// * `recurring_value` - Number of billing cycles to apply (None = forever)
    pub async fn create_limited_percentage_coupon(
        &self,
        code: &str,
        percentage: u32,
        recurring_value: Option<i32>,
    ) -> CouponId {
        self.create_coupon(CouponNew {
            code: code.to_string(),
            description: format!(
                "{}% discount for {} cycles",
                percentage,
                recurring_value.map_or("unlimited".to_string(), |v| v.to_string())
            ),
            tenant_id: TENANT_ID,
            discount: CouponDiscount::Percentage(Decimal::from(percentage)),
            expires_at: None,
            redemption_limit: None,
            recurring_value,
            reusable: false,
            plan_ids: vec![],
        })
        .await
    }

    /// A percentage coupon for the billing periods starting within `months` of the first paid
    /// period.
    pub async fn create_months_coupon(&self, code: &str, percentage: u32, months: i32) -> CouponId {
        self.store()
            .create_coupon_with_terms(
                common_domain::actor::Actor::System,
                CouponNew {
                    code: code.to_string(),
                    description: format!("{percentage}% off for {months} months"),
                    tenant_id: TENANT_ID,
                    discount: CouponDiscount::Percentage(Decimal::from(percentage)),
                    expires_at: None,
                    redemption_limit: None,
                    recurring_value: None,
                    reusable: false,
                    plan_ids: vec![],
                },
                CouponTerms {
                    duration_months: Some(months),
                    ..Default::default()
                },
            )
            .await
            .expect("Failed to create coupon")
            .id
    }

    /// Changes the months of `coupon_id`: `None` makes it last forever.
    pub async fn set_coupon_months(&self, coupon_id: CouponId, months: Option<i32>) {
        self.store()
            .update_coupon(
                common_domain::actor::Actor::System,
                CouponPatch {
                    id: coupon_id,
                    tenant_id: TENANT_ID,
                    description: None,
                    discount: None,
                    plan_ids: None,
                    add_on_ids: None,
                    eligibility: None,
                    valid_from: None,
                    expires_at: None,
                    fixed_amounts: None,
                    duration_months: Some(months),
                },
            )
            .await
            .expect("Failed to change the coupon's months");
    }

    /// Create a coupon restricted to specific plans.
    ///
    /// # Arguments
    /// * `code` - Coupon code
    /// * `percentage` - Discount percentage
    /// * `plan_ids` - List of plan IDs this coupon applies to
    pub async fn create_plan_restricted_coupon(
        &self,
        code: &str,
        percentage: u32,
        plan_ids: Vec<PlanId>,
    ) -> CouponId {
        self.create_coupon(CouponNew {
            code: code.to_string(),
            description: format!("{}% discount (plan restricted)", percentage),
            tenant_id: TENANT_ID,
            discount: CouponDiscount::Percentage(Decimal::from(percentage)),
            expires_at: None,
            redemption_limit: None,
            recurring_value: None,
            reusable: false,
            plan_ids,
        })
        .await
    }

    /// Create a reusable percentage coupon (can be used by same customer multiple times).
    pub async fn create_reusable_coupon(&self, code: &str, percentage: u32) -> CouponId {
        self.create_coupon(CouponNew {
            code: code.to_string(),
            description: format!("{}% discount (reusable)", percentage),
            tenant_id: TENANT_ID,
            discount: CouponDiscount::Percentage(Decimal::from(percentage)),
            expires_at: None,
            redemption_limit: None,
            recurring_value: None,
            reusable: true,
            plan_ids: vec![],
        })
        .await
    }

    /// Create a coupon with a redemption limit (max number of subscriptions).
    pub async fn create_limited_redemption_coupon(
        &self,
        code: &str,
        percentage: u32,
        redemption_limit: i32,
    ) -> CouponId {
        self.create_coupon(CouponNew {
            code: code.to_string(),
            description: format!("{}% discount (limit: {})", percentage, redemption_limit),
            tenant_id: TENANT_ID,
            discount: CouponDiscount::Percentage(Decimal::from(percentage)),
            expires_at: None,
            redemption_limit: Some(redemption_limit),
            recurring_value: None,
            reusable: true, // reusable so limit is the only constraint
            plan_ids: vec![],
        })
        .await
    }

    /// Create a coupon with full control over all fields.
    pub async fn create_coupon(&self, coupon: CouponNew) -> CouponId {
        self.store()
            .create_coupon(common_domain::actor::Actor::System, coupon)
            .await
            .expect("Failed to create coupon")
            .id
    }

    /// Disable a coupon.
    pub async fn disable_coupon(&self, coupon_id: CouponId) {
        self.store()
            .update_coupon_status(
                common_domain::actor::Actor::System,
                CouponStatusPatch {
                    id: coupon_id,
                    tenant_id: TENANT_ID,
                    archived_at: None,
                    disabled: Some(true),
                },
            )
            .await
            .expect("Failed to disable coupon");
    }

    /// Archive a coupon.
    pub async fn archive_coupon(&self, coupon_id: CouponId) {
        self.store()
            .update_coupon_status(
                common_domain::actor::Actor::System,
                CouponStatusPatch {
                    id: coupon_id,
                    tenant_id: TENANT_ID,
                    archived_at: Some(Some(common_clock::now().naive_utc())),
                    disabled: None,
                },
            )
            .await
            .expect("Failed to archive coupon");
    }
}
