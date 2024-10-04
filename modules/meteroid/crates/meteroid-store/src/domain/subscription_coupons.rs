use chrono::NaiveDateTime;
use diesel_models::subscription_coupons::SubscriptionCouponRow;
use o2o::o2o;
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

#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[from_owned(SubscriptionCouponRow)]
#[owned_into(SubscriptionCouponRow)]
pub struct SubscriptionCoupon {
    pub id: Uuid,
    pub subscription_id: Uuid,
    pub coupon_id: Uuid,
    pub created_at: NaiveDateTime,
}
