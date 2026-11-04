use chrono::NaiveDateTime;
use common_domain::ids::{PlanVersionId, ProductId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

use crate::enums::AdjustmentKindEnum;

#[derive(Queryable, Debug, Identifiable, Selectable, Clone)]
#[diesel(table_name = crate::schema::plan_version_adjustment)]
#[diesel(primary_key(plan_version_id, kind))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanVersionAdjustmentRow {
    pub plan_version_id: PlanVersionId,
    pub kind: AdjustmentKindEnum,
    pub amount_subunit: i64,
    pub includes_all_components: bool,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::plan_version_adjustment)]
pub struct PlanVersionAdjustmentRowNew {
    pub plan_version_id: PlanVersionId,
    pub kind: AdjustmentKindEnum,
    pub amount_subunit: i64,
    pub includes_all_components: bool,
}

#[derive(Queryable, Debug, Identifiable, Selectable, Clone)]
#[diesel(table_name = crate::schema::plan_version_adjustment_component)]
#[diesel(primary_key(plan_version_id, kind, product_id))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanVersionAdjustmentComponentRow {
    pub plan_version_id: PlanVersionId,
    pub kind: AdjustmentKindEnum,
    pub product_id: ProductId,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::plan_version_adjustment_component)]
pub struct PlanVersionAdjustmentComponentRowNew {
    pub plan_version_id: PlanVersionId,
    pub kind: AdjustmentKindEnum,
    pub product_id: ProductId,
}
