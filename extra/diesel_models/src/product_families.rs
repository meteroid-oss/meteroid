

use chrono::NaiveDateTime;
use uuid::Uuid;


use diesel::{Identifiable, Queryable};



#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::product_family)]
pub struct ProductFamily {
    pub id: Uuid,
    pub name: String,
    pub external_id: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
}