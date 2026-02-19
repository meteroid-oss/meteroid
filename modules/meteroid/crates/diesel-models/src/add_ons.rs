use chrono::NaiveDateTime;
use common_domain::ids::{AddOnId, PlanVersionId, PriceId, ProductId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AddOnRow {
    pub id: AddOnId,
    pub name: String,
    pub tenant_id: TenantId,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub plan_version_id: Option<PlanVersionId>,
    pub product_id: Option<ProductId>,
    pub price_id: Option<PriceId>,
}

#[derive(Debug, Default, Insertable)]
#[diesel(table_name = crate::schema::add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AddOnRowNew {
    pub id: AddOnId,
    pub name: String,
    pub tenant_id: TenantId,
    pub plan_version_id: Option<PlanVersionId>,
    pub product_id: Option<ProductId>,
    pub price_id: Option<PriceId>,
}

#[derive(AsChangeset)]
#[diesel(table_name = crate::schema::add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id, tenant_id))]
pub struct AddOnRowPatch {
    pub id: AddOnId,
    pub tenant_id: TenantId,
    pub name: Option<String>,
    pub plan_version_id: Option<Option<PlanVersionId>>,
    pub product_id: Option<Option<ProductId>>,
    pub price_id: Option<Option<PriceId>>,
    pub updated_at: NaiveDateTime,
}
