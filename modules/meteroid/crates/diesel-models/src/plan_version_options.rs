use chrono::NaiveDateTime;
use common_domain::ids::{PlanOptionId, PlanVersionId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Clone, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::plan_version_option)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanVersionOptionRow {
    pub id: PlanOptionId,
    pub tenant_id: TenantId,
    pub plan_version_id: PlanVersionId,
    pub name: String,
    pub alias: String,
    pub position: i32,
    pub created_at: NaiveDateTime,
    pub hidden: bool,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::plan_version_option)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanVersionOptionRowNew {
    pub id: PlanOptionId,
    pub tenant_id: TenantId,
    pub plan_version_id: PlanVersionId,
    pub name: String,
    pub alias: String,
    pub position: i32,
    pub hidden: bool,
}
