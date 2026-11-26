use crate::enums::AppCountOpEnum;
use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, FeatureId, ResourceId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};
use rust_decimal::Decimal;

/// A count reported by the app for one path: `''` for the customer, else `<type code>=<key value>`.
#[derive(Debug, Clone, Identifiable, Queryable, Selectable, Insertable, AsChangeset)]
#[diesel(table_name = crate::schema::entitlement_app_count)]
#[diesel(primary_key(tenant_id, customer_id, feature_id, path))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EntitlementAppCountRow {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub feature_id: FeatureId,
    pub path: String,
    pub resource_id: Option<ResourceId>,
    pub count: Decimal,
    pub last_set_value: Option<Decimal>,
    pub last_set_at: Option<DateTime<Utc>>,
    pub last_set_key: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::entitlement_app_count_op)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EntitlementAppCountOpRow {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub feature_id: FeatureId,
    pub idempotency_key: String,
    pub path: String,
    pub op: AppCountOpEnum,
    pub amount: Decimal,
    pub at: DateTime<Utc>,
    pub result_count: Decimal,
    pub request_hash: String,
    pub created_at: DateTime<Utc>,
}

/// The adds of a path after an instant, from the op log.
#[derive(Debug, Clone, Queryable)]
pub struct AppCountAddsRow {
    pub count: i64,
    pub sum: Option<Decimal>,
    pub last_at: Option<DateTime<Utc>>,
}
