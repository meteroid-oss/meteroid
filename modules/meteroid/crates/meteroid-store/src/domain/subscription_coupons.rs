use crate::domain::coupons::{Coupon, CouponDiscount};
use crate::errors::StoreErrorReport;
use chrono::NaiveDateTime;
use diesel_models::applied_coupons::{
    AppliedCouponDetailedRow, AppliedCouponForDisplayRow, AppliedCouponRow,
};
use o2o::o2o;
use rust_decimal::Decimal;
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
#[from_owned(AppliedCouponRow)]
#[owned_into(AppliedCouponRow)]
pub struct AppliedCoupon {
    pub id: Uuid,
    pub coupon_id: Uuid,
    pub customer_id: Uuid,
    pub subscription_id: Uuid,
    pub is_active: bool,
    pub applied_amount: Option<Decimal>,
    pub applied_count: Option<i32>,
    pub last_applied_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, o2o)]
#[from_owned(AppliedCouponForDisplayRow)]
pub struct AppliedCouponForDisplay {
    pub id: Uuid,
    pub coupon_id: Uuid,
    pub customer_id: Uuid,
    pub customer_local_id: String,
    pub customer_name: String,
    pub subscription_id: Uuid,
    pub plan_id: Uuid,
    pub plan_local_id: String,
    pub plan_version: i32,
    pub plan_name: String,
    pub is_active: bool,
    pub applied_amount: Option<Decimal>,
    pub applied_count: Option<i32>,
    pub last_applied_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone)]
pub struct AppliedCouponDetailed {
    pub coupon: Coupon,
    pub applied_coupon: AppliedCoupon,
}

impl AppliedCouponDetailed {
    pub fn is_invoice_applicable(&self) -> bool {
        self.applied_coupon.is_active
            && !self.reached_recurring_limit()
            && !self.amount_is_fully_consumed()
    }

    fn reached_recurring_limit(&self) -> bool {
        self.coupon
            .recurring_value
            .map(|x| x <= self.applied_coupon.applied_count.unwrap_or(0))
            .unwrap_or(false)
    }

    fn amount_is_fully_consumed(&self) -> bool {
        match &self.coupon.discount {
            CouponDiscount::Percentage(_) => false,
            CouponDiscount::Fixed { amount, .. } => {
                // todo currency conversion?
                let fully_consumed = &self
                    .applied_coupon
                    .applied_amount
                    .unwrap_or(Decimal::from(0))
                    >= amount;

                let applies_once = self.coupon.applies_once();

                applies_once && fully_consumed
            }
        }
    }
}

impl TryInto<AppliedCouponDetailed> for AppliedCouponDetailedRow {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<AppliedCouponDetailed, Self::Error> {
        let coupon: Coupon = self.coupon.try_into()?;
        let applied_coupon: AppliedCoupon = self.applied_coupon.into();

        Ok(AppliedCouponDetailed {
            coupon,
            applied_coupon,
        })
    }
}
