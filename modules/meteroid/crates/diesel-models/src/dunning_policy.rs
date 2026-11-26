use chrono::{DateTime, Utc};
use common_domain::ids::TenantId;
use diesel::{Insertable, Queryable, Selectable};

use crate::enums::DunningExhaustedActionEnum;

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::dunning_policy)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DunningPolicyRow {
    pub tenant_id: TenantId,
    pub retry_days: Vec<Option<i32>>,
    pub exhausted_action: DunningExhaustedActionEnum,
    pub updated_at: DateTime<Utc>,
}
