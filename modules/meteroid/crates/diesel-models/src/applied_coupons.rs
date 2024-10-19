use diesel::{Identifiable, Insertable, Queryable, Selectable};
use rust_decimal::Decimal;

#[derive(Debug, Queryable, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::applied_coupon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AppliedCouponRow {
    pub id: Uuid,
    pub coupon_id: Uuid,
    pub customer_id: Uuid,
    pub subscription_id: Uuid,
    pub is_active: bool,
    pub applied_amount: Option<Decimal>,
    pub applied_count: Option<i32>,
    pub last_applied_at: Option<chrono::NaiveDateTime>,
    pub created_at: chrono::NaiveDateTime,
}

use uuid::Uuid;

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::applied_coupon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AppliedCouponRowNew {
    pub id: Uuid,
    pub coupon_id: Uuid,
    pub customer_id: Uuid,
    pub subscription_id: Uuid,
    pub is_active: bool,
    pub applied_amount: Option<Decimal>,
    pub applied_count: Option<i32>,
    pub last_applied_at: Option<chrono::NaiveDateTime>,
}
