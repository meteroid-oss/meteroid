use chrono::NaiveDateTime;
use common_domain::ids::{AddOnId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AddOnRow {
    pub id: AddOnId,
    pub name: String,
    pub fee: serde_json::Value,
    pub tenant_id: TenantId,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Default, Insertable)]
#[diesel(table_name = crate::schema::add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AddOnRowNew {
    pub id: AddOnId,
    pub name: String,
    pub fee: serde_json::Value,
    pub tenant_id: TenantId,
}

#[derive(AsChangeset)]
#[diesel(table_name = crate::schema::add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id, tenant_id))]
pub struct AddOnRowPatch {
    pub id: AddOnId,
    pub tenant_id: TenantId,
    pub name: Option<String>,
    pub fee: Option<serde_json::Value>,
    pub updated_at: NaiveDateTime,
}
