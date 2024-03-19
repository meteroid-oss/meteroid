use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::schema::tenant;
use diesel::{Identifiable, Insertable, Queryable};

#[derive(Clone, Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::tenant)]
pub struct Tenant {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub organization_id: Uuid,
    pub currency: String,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::tenant)]
pub struct TenantNew {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub organization_id: Uuid,
    pub currency: String,
}
