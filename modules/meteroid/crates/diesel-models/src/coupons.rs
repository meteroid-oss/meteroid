use chrono::NaiveDateTime;
use common_domain::ids::{CouponId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::coupon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CouponRow {
    pub id: CouponId,
    pub code: String,
    pub description: String,
    pub tenant_id: TenantId,
    pub discount: serde_json::Value,
    pub expires_at: Option<NaiveDateTime>,
    pub redemption_limit: Option<i32>,
    pub recurring_value: Option<i32>,
    pub reusable: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub redemption_count: i32,
    pub last_redemption_at: Option<NaiveDateTime>,
    pub disabled: bool,
    pub archived_at: Option<NaiveDateTime>,
}

#[derive(Debug, Default, Insertable)]
#[diesel(table_name = crate::schema::coupon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CouponRowNew {
    pub id: CouponId,
    pub code: String,
    pub description: String,
    pub tenant_id: TenantId,
    pub discount: serde_json::Value,
    pub expires_at: Option<NaiveDateTime>,
    pub redemption_limit: Option<i32>,
    pub recurring_value: Option<i32>,
    pub reusable: bool,
}

#[derive(AsChangeset)]
#[diesel(table_name = crate::schema::coupon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id, tenant_id))]
pub struct CouponRowPatch {
    pub id: CouponId,
    pub tenant_id: TenantId,
    pub description: Option<String>,
    pub discount: Option<serde_json::Value>,
    pub updated_at: NaiveDateTime,
}

#[derive(AsChangeset)]
#[diesel(table_name = crate::schema::coupon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id, tenant_id))]
pub struct CouponStatusRowPatch {
    pub id: CouponId,
    pub tenant_id: TenantId,
    pub archived_at: Option<Option<NaiveDateTime>>,
    pub disabled: Option<bool>,
}

pub enum CouponFilter {
    ALL,
    ACTIVE,
    INACTIVE,
    ARCHIVED,
}
