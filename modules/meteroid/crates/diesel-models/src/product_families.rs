use chrono::NaiveDateTime;
use uuid::Uuid;

use diesel::{Identifiable, Insertable, Queryable};

#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::product_family)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProductFamilyRow {
    pub id: Uuid,
    pub name: String,
    pub local_id: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::product_family)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProductFamilyRowNew {
    pub id: Uuid,
    pub name: String,
    pub local_id: String,
    pub tenant_id: Uuid,
}
