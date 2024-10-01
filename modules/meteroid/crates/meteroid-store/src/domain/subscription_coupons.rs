use crate::domain::coupons::CouponDiscount;
use crate::errors::StoreError;
use chrono::NaiveDateTime;
use diesel_models::subscription_coupons::SubscriptionCouponRow;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct CreateSubscriptionCoupon {
    pub coupon_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct CreateSubscriptionCoupons {
    pub coupons: Vec<CreateSubscriptionCoupon>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionCoupon {
    pub id: Uuid,
    pub subscription_id: Uuid,
    pub coupon_id: Uuid,
    pub coupon_code: String,
    pub coupon_description: String,
    pub coupon_discount: CouponDiscount,
    pub coupon_expires_at: Option<NaiveDateTime>,
    pub coupon_redemption_limit: Option<i32>, // max number of subscriptions it can be applied to
    pub coupon_recurring_value: i32,          // 1 once, -1 infinite, > 1 number of times
    pub coupon_reusable: bool, // can it be applied to multiple subscriptions of the same customer
}

impl TryInto<SubscriptionCoupon> for SubscriptionCouponRow {
    type Error = StoreError;

    fn try_into(self) -> Result<SubscriptionCoupon, Self::Error> {
        let discount: CouponDiscount = serde_json::from_value(self.coupon_discount)
            .map_err(|e| StoreError::SerdeError("coupon discount".to_string(), e))?;

        Ok(SubscriptionCoupon {
            id: self.id,
            subscription_id: self.subscription_id,
            coupon_id: self.coupon_id,
            coupon_code: self.coupon_code,
            coupon_description: self.coupon_description,
            coupon_discount: discount,
            coupon_expires_at: self.coupon_expires_at,
            coupon_redemption_limit: self.coupon_redemption_limit,
            coupon_recurring_value: self.coupon_recurring_value,
            coupon_reusable: self.coupon_reusable,
        })
    }
}
