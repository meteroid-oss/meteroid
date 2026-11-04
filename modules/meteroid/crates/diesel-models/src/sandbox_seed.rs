use chrono::NaiveDateTime;
use common_domain::ids::TenantId;
use diesel::{Queryable, Selectable};

use crate::enums::SandboxSeedStatusEnum;

#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = crate::schema::sandbox_seed)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SandboxSeedRow {
    pub tenant_id: TenantId,
    pub status: SandboxSeedStatusEnum,
    pub failure_reason: Option<String>,
    pub started_at: NaiveDateTime,
    pub finished_at: Option<NaiveDateTime>,
}
