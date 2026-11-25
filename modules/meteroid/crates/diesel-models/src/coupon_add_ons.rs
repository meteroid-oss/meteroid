use common_domain::ids::{AddOnId, CouponId};
use diesel::{Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Selectable)]
#[diesel(table_name = crate::schema::coupon_add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CouponAddOnRow {
    pub coupon_id: CouponId,
    pub add_on_id: AddOnId,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::coupon_add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CouponAddOnRowNew {
    pub coupon_id: CouponId,
    pub add_on_id: AddOnId,
}
