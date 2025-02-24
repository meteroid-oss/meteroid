use crate::coupons::CouponRow;
use common_domain::ids::CustomerId;
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use rust_decimal::Decimal;
use uuid::Uuid;

#[derive(Debug, Queryable, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::applied_coupon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AppliedCouponRow {
    pub id: Uuid,
    pub coupon_id: Uuid,
    pub customer_id: CustomerId,
    pub subscription_id: Uuid,
    pub is_active: bool,
    pub applied_amount: Option<Decimal>,
    pub applied_count: Option<i32>,
    pub last_applied_at: Option<chrono::NaiveDateTime>,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Queryable, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::applied_coupon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AppliedCouponForDisplayRow {
    pub id: Uuid,
    pub coupon_id: Uuid,
    pub customer_id: CustomerId,
    #[diesel(select_expression = crate::schema::customer::name)]
    #[diesel(select_expression_type = crate::schema::customer::name)]
    pub customer_name: String,
    pub subscription_id: Uuid,
    #[diesel(select_expression = crate::schema::plan::id)]
    #[diesel(select_expression_type = crate::schema::plan::id)]
    pub plan_id: Uuid,
    #[diesel(select_expression = crate::schema::plan::local_id)]
    #[diesel(select_expression_type = crate::schema::plan::local_id)]
    pub plan_local_id: String,
    #[diesel(select_expression = crate::schema::plan_version::version)]
    #[diesel(select_expression_type = crate::schema::plan_version::version)]
    pub plan_version: i32,
    #[diesel(select_expression = crate::schema::plan::name)]
    #[diesel(select_expression_type = crate::schema::plan::name)]
    pub plan_name: String,
    pub is_active: bool,
    pub applied_amount: Option<Decimal>,
    pub applied_count: Option<i32>,
    pub last_applied_at: Option<chrono::NaiveDateTime>,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::applied_coupon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AppliedCouponRowNew {
    pub id: Uuid,
    pub coupon_id: Uuid,
    pub customer_id: CustomerId,
    pub subscription_id: Uuid,
    pub is_active: bool,
    pub applied_amount: Option<Decimal>,
    pub applied_count: Option<i32>,
    pub last_applied_at: Option<chrono::NaiveDateTime>,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AppliedCouponDetailedRow {
    #[diesel(embed)]
    pub coupon: CouponRow,
    #[diesel(embed)]
    pub applied_coupon: AppliedCouponRow,
}
