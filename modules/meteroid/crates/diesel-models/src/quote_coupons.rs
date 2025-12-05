use common_domain::ids::{CouponId, QuoteCouponId, QuoteId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::quote_coupon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteCouponRow {
    pub id: QuoteCouponId,
    pub quote_id: QuoteId,
    pub coupon_id: CouponId,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::quote_coupon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteCouponRowNew {
    pub id: QuoteCouponId,
    pub quote_id: QuoteId,
    pub coupon_id: CouponId,
}
