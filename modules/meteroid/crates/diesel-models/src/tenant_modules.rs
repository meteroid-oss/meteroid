use chrono::{DateTime, Utc};
use common_domain::ids::TenantId;
use diesel::{Insertable, Queryable, Selectable};

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::tenant_module)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TenantModuleRow {
    pub tenant_id: TenantId,
    pub module: String,
    pub enabled_at: DateTime<Utc>,
}
