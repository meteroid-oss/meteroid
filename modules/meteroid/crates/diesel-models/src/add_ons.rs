use chrono::NaiveDateTime;
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AddOnRow {
    pub id: Uuid,
    pub name: String,
    pub fee: serde_json::Value,
    pub tenant_id: Uuid,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Default, Insertable)]
#[diesel(table_name = crate::schema::add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AddOnRowNew {
    pub id: Uuid,
    pub name: String,
    pub fee: serde_json::Value,
    pub tenant_id: Uuid,
}

#[derive(AsChangeset)]
#[diesel(table_name = crate::schema::add_on)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id, tenant_id))]
pub struct AddOnRowPatch {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: Option<String>,
    pub fee: Option<serde_json::Value>,
    pub updated_at: NaiveDateTime,
}
