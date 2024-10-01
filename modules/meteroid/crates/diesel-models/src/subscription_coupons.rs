use chrono::NaiveDateTime;
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::subscription_coupon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionCouponRow {
    pub id: Uuid,
    pub subscription_id: Uuid,
    pub coupon_id: Uuid,
    pub coupon_code: String,
    pub coupon_description: String,
    pub coupon_discount: serde_json::Value,
    pub coupon_expires_at: Option<NaiveDateTime>,
    pub coupon_redemption_limit: Option<i32>,
    pub coupon_recurring_value: i32,
    pub coupon_reusable: bool,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::subscription_coupon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionCouponRowNew {
    pub id: Uuid,
    pub subscription_id: Uuid,
    pub coupon_id: Uuid,
    pub coupon_code: String,
    pub coupon_description: String,
    pub coupon_discount: serde_json::Value,
    pub coupon_expires_at: Option<NaiveDateTime>,
    pub coupon_redemption_limit: Option<i32>,
    pub coupon_recurring_value: i32,
    pub coupon_reusable: bool,
}
