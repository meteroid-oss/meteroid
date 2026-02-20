use chrono::NaiveDateTime;
use common_domain::ids::{AddOnId, PlanVersionAddOnId, PlanVersionId, PriceId, TenantId};
use diesel_models::plan_version_add_ons::{PlanVersionAddOnRow, PlanVersionAddOnRowNew};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PlanVersionAddOn {
    pub id: PlanVersionAddOnId,
    pub plan_version_id: PlanVersionId,
    pub add_on_id: AddOnId,
    pub price_id: Option<PriceId>,
    pub self_serviceable: Option<bool>,
    pub max_instances_per_subscription: Option<i32>,
    pub tenant_id: TenantId,
    pub created_at: NaiveDateTime,
}

impl From<PlanVersionAddOnRow> for PlanVersionAddOn {
    fn from(row: PlanVersionAddOnRow) -> Self {
        PlanVersionAddOn {
            id: PlanVersionAddOnId::from(row.id),
            plan_version_id: row.plan_version_id,
            add_on_id: row.add_on_id,
            price_id: row.price_id,
            self_serviceable: row.self_serviceable,
            max_instances_per_subscription: row.max_instances_per_subscription,
            tenant_id: row.tenant_id,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanVersionAddOnNew {
    pub plan_version_id: PlanVersionId,
    pub add_on_id: AddOnId,
    pub price_id: Option<PriceId>,
    pub self_serviceable: Option<bool>,
    pub max_instances_per_subscription: Option<i32>,
    pub tenant_id: TenantId,
}

impl From<PlanVersionAddOnNew> for PlanVersionAddOnRowNew {
    fn from(new: PlanVersionAddOnNew) -> Self {
        PlanVersionAddOnRowNew {
            id: Uuid::now_v7(),
            plan_version_id: new.plan_version_id,
            add_on_id: new.add_on_id,
            price_id: new.price_id,
            self_serviceable: new.self_serviceable,
            max_instances_per_subscription: new.max_instances_per_subscription,
            tenant_id: new.tenant_id,
        }
    }
}
