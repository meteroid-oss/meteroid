use chrono::NaiveDateTime;
use common_domain::ids::{CouponId, OfferId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::coupon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CouponRow {
    pub id: CouponId,
    /// `None` only from release 2 of offers (ADR 0019): a grant's definition or an offer's code.
    pub code: Option<String>,
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
    pub eligibility: Option<serde_json::Value>,
    pub valid_from: Option<NaiveDateTime>,
    /// `{"EUR": "10.00"}`: a fixed discount in currencies other than its own.
    pub fixed_amounts: Option<serde_json::Value>,
    /// Periods starting within this many months of the first paid period; `recurring_value`
    /// counts invoices instead (older ones).
    pub duration_months: Option<i32>,
    /// The offer the code gives; `None` for a grant's own definition, or one not wrapped yet.
    pub offer_id: Option<OfferId>,
    /// Trigger-maintained analytics ingestion marker; never written by app code.
    pub store_committed_at: chrono::NaiveDateTime,
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
    pub eligibility: Option<serde_json::Value>,
    pub valid_from: Option<NaiveDateTime>,
    pub fixed_amounts: Option<serde_json::Value>,
    pub duration_months: Option<i32>,
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
    pub eligibility: Option<Option<serde_json::Value>>,
    pub valid_from: Option<Option<NaiveDateTime>>,
    pub expires_at: Option<Option<NaiveDateTime>>,
    pub fixed_amounts: Option<Option<serde_json::Value>>,
    /// `Some(None)` makes it last forever.
    pub duration_months: Option<Option<i32>>,
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
