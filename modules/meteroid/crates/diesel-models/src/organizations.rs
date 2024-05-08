use chrono::NaiveDateTime;
use uuid::Uuid;

use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::organization)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub invite_link_hash: Option<String>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::organization)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OrganizationNew {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
}
