use chrono::{DateTime, Utc};
use common_domain::ids::TenantId;
use diesel::{Insertable, Queryable, Selectable};

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::api_idempotency_key)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ApiIdempotencyKeyRow {
    pub tenant_id: TenantId,
    pub operation: String,
    pub idempotency_key: String,
    pub request_fingerprint: Vec<u8>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::api_idempotency_key)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ApiIdempotencyKeyRowNew {
    pub tenant_id: TenantId,
    pub operation: String,
    pub idempotency_key: String,
    pub request_fingerprint: Vec<u8>,
}
