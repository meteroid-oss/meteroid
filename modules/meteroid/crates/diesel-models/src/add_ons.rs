use chrono::NaiveDateTime;
use common_domain::ids::{AddOnId, PriceId, ProductId, TenantId};
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
    pub product_id: ProductId,
    pub price_id: PriceId,
    pub description: Option<String>,
    pub self_serviceable: bool,
    pub max_instances_per_subscription: Option<i32>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AddOnRowNew {
    pub id: AddOnId,
    pub name: String,
    pub tenant_id: TenantId,
    pub product_id: ProductId,
    pub price_id: PriceId,
    pub description: Option<String>,
    pub self_serviceable: bool,
    pub max_instances_per_subscription: Option<i32>,
}

#[derive(AsChangeset)]
#[diesel(table_name = crate::schema::add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id, tenant_id))]
pub struct AddOnRowPatch {
    pub id: AddOnId,
    pub tenant_id: TenantId,
    pub name: Option<String>,
    pub price_id: Option<PriceId>,
    pub description: Option<Option<String>>,
    pub self_serviceable: Option<bool>,
    pub max_instances_per_subscription: Option<Option<i32>>,
    pub updated_at: NaiveDateTime,
}
