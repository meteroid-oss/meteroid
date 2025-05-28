use uuid::Uuid;

use super::plan_versions::PlanVersionRow;
use crate::enums::BillingPeriodEnum;
use common_domain::ids::PlanVersionId;
use diesel::{AsChangeset, Associations, Insertable, Queryable, Selectable};

#[derive(Queryable, Associations, Selectable, Debug)]
#[diesel(table_name = crate::schema::schedule)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(belongs_to(PlanVersionRow, foreign_key = plan_version_id))]
pub struct ScheduleRow {
    pub id: Uuid,
    pub billing_period: BillingPeriodEnum,
    pub plan_version_id: PlanVersionId,
    pub ramps: serde_json::Value,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::schedule)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ScheduleRowNew {
    pub id: Uuid,
    pub billing_period: BillingPeriodEnum,
    pub plan_version_id: PlanVersionId,
    pub ramps: serde_json::Value,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::schedule)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SchedulePatchRow {
    pub id: Uuid,
    pub ramps: Option<serde_json::Value>,
}
