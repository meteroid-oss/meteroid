use chrono::NaiveDateTime;
use common_domain::ids::{AddOnId, PlanVersionId, PriceId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::plan_version_add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanVersionAddOnRow {
    pub id: Uuid,
    pub plan_version_id: PlanVersionId,
    pub add_on_id: AddOnId,
    pub price_id: Option<PriceId>,
    pub self_serviceable: Option<bool>,
    pub max_instances_per_subscription: Option<i32>,
    pub tenant_id: TenantId,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::plan_version_add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanVersionAddOnRowNew {
    pub id: Uuid,
    pub plan_version_id: PlanVersionId,
    pub add_on_id: AddOnId,
    pub price_id: Option<PriceId>,
    pub self_serviceable: Option<bool>,
    pub max_instances_per_subscription: Option<i32>,
    pub tenant_id: TenantId,
}
