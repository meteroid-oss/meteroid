use crate::domain::coupons::Coupon;
use crate::errors::StoreErrorReport;
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{AppliedCouponId, CouponId, CustomerId, PlanId, SubscriptionId};
use diesel_models::applied_coupons::{
    AppliedCouponDetailedRow, AppliedCouponForDisplayRow, AppliedCouponRow,
};
use o2o::o2o;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct CreateSubscriptionCoupon {
    pub coupon_id: CouponId,
}

#[derive(Debug, Clone)]
pub struct CreateSubscriptionCoupons {
    pub coupons: Vec<CreateSubscriptionCoupon>,
}

#[derive(Debug, Clone, Serialize, Deserialize, o2o)]
#[from_owned(AppliedCouponRow)]
#[owned_into(AppliedCouponRow)]
#[ghosts(
    fixed_amount: {None},
    fixed_currency: {None},
    add_on_ids: {None},
    duration_months: {None}
)]
pub struct AppliedCoupon {
    pub id: AppliedCouponId,
    pub coupon_id: CouponId,
    pub customer_id: CustomerId,
    pub subscription_id: SubscriptionId,
    pub is_active: bool,
    pub applied_amount: Option<Decimal>,
    pub applied_count: Option<i32>,
    pub last_applied_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    /// The first period start it discounts, from which a coupon lasting months counts them: the
    /// first paid period's when redeemed with the subscription, the next period's when added to a
    /// running one.
    pub effective_from: NaiveDate,
    /// Added to a running subscription: the first service period it discounts, the lines of
    /// earlier periods billed in arrears after it keeping their price. `None`: every line.
    pub discounts_from: Option<NaiveDate>,
}

#[derive(Debug, Clone, o2o)]
#[from_owned(AppliedCouponForDisplayRow)]
pub struct AppliedCouponForDisplay {
    pub id: AppliedCouponId,
    pub coupon_id: CouponId,
    pub customer_id: CustomerId,
    pub customer_name: String,
    pub subscription_id: SubscriptionId,
    pub plan_id: PlanId,
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
    /// Credit coupons pay out through the customer's wallet, never as an invoice discount. A
    /// fixed coupon is limited by its invoices only, never by what it gave so far (R-089).
    pub fn is_invoice_applicable(&self) -> bool {
        !self.coupon.discount.is_credit()
            && self.applied_coupon.is_active
            && !self.reached_recurring_limit()
    }

    /// Whether it discounts a line of the service period starting on `period_start`: always, but
    /// for a coupon lasting months, past which none is (R-089).
    pub fn covers(&self, period_start: NaiveDate) -> bool {
        self.lasts_until().is_none_or(|end| period_start < end)
    }

    /// The day a coupon lasting months ends on this subscription: its first paid period's start
    /// plus the months it was redeemed for.
    pub fn lasts_until(&self) -> Option<NaiveDate> {
        self.coupon.lasts_until(self.applied_coupon.effective_from)
    }

    /// Invoices are counted only by coupons made before durations in months.
    fn reached_recurring_limit(&self) -> bool {
        self.coupon
            .recurring_value
            .is_some_and(|x| x <= self.applied_coupon.applied_count.unwrap_or(0))
    }
}

impl TryInto<AppliedCouponDetailed> for AppliedCouponDetailedRow {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<AppliedCouponDetailed, Self::Error> {
        let row = &self.applied_coupon;
        let fixed = row.fixed_currency.clone().zip(row.fixed_amount);
        let add_on_ids = row
            .add_on_ids
            .clone()
            .map(|ids| ids.into_iter().flatten().collect());
        let duration_months = row.duration_months;
        let mut coupon = TryInto::<Coupon>::try_into(self.coupon)?.as_redeemed(fixed, add_on_ids);
        // The months it was redeemed for: an edit of the coupon reaches only later redemptions.
        coupon.duration_months = duration_months;
        let applied_coupon: AppliedCoupon = self.applied_coupon.into();

        Ok(AppliedCouponDetailed {
            coupon,
            applied_coupon,
        })
    }
}
