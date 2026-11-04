use chrono::NaiveDateTime;
use diesel::sql_types::{Array, BigInt, Integer, Text};
use diesel::{Queryable, QueryableByName, Selectable};

/// This instance's anonymous telemetry identity (ADR 0010). At most one row.
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::instance_telemetry)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InstanceTelemetryRow {
    pub id: bool,
    pub instance_id: uuid::Uuid,
    pub installed_at: NaiveDateTime,
    pub first_version: String,
    pub last_sent_at: Option<NaiveDateTime>,
}

/// Instance-wide counts; business objects are counted in production tenants only.
#[derive(Debug, Clone, QueryableByName)]
pub struct InstanceUsageRow {
    #[diesel(sql_type = BigInt)]
    pub organizations: i64,
    #[diesel(sql_type = BigInt)]
    pub tenants_live: i64,
    #[diesel(sql_type = BigInt)]
    pub tenants_other: i64,
    #[diesel(sql_type = BigInt)]
    pub users: i64,
    #[diesel(sql_type = BigInt)]
    pub customers: i64,
    #[diesel(sql_type = BigInt)]
    pub subscriptions_active: i64,
    #[diesel(sql_type = BigInt)]
    pub plans_published: i64,
    #[diesel(sql_type = BigInt)]
    pub billable_metrics: i64,
    #[diesel(sql_type = BigInt)]
    pub invoices_finalized_30d: i64,
    #[diesel(sql_type = Array<Text>)]
    pub currencies: Vec<String>,
    #[diesel(sql_type = Array<Text>)]
    pub integrations: Vec<String>,
    #[diesel(sql_type = Integer)]
    pub postgres_major: i32,
}
