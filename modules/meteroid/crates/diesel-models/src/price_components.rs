use common_domain::ids::{BillableMetricId, PlanVersionId, PriceComponentId, ProductId};
use diesel::{AsChangeset, Associations, Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, AsChangeset, Selectable, Associations, Clone)]
#[diesel(table_name = crate::schema::price_component)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(belongs_to(crate::plan_versions::PlanVersionRow, foreign_key=plan_version_id))]
pub struct PriceComponentRow {
    pub id: PriceComponentId,
    pub name: String,
    pub fee: serde_json::Value,
    pub plan_version_id: PlanVersionId,
    pub product_id: Option<ProductId>,
    pub billable_metric_id: Option<BillableMetricId>,
}

#[derive(Debug, Default, Insertable)]
#[diesel(table_name = crate::schema::price_component)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PriceComponentRowNew {
    pub id: PriceComponentId,
    pub name: String,
    pub fee: serde_json::Value,
    pub plan_version_id: PlanVersionId,
    pub product_id: Option<ProductId>,
    pub billable_metric_id: Option<BillableMetricId>,
}

// the changeset one
