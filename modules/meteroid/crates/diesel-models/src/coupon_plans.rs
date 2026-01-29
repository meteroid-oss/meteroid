use common_domain::ids::{CouponId, PlanId};
use diesel::{Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Selectable)]
#[diesel(table_name = crate::schema::coupon_plan)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CouponPlanRow {
    pub coupon_id: CouponId,
    pub plan_id: PlanId,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::coupon_plan)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CouponPlanRowNew {
    pub coupon_id: CouponId,
    pub plan_id: PlanId,
}
