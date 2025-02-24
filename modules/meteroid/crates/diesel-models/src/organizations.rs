use chrono::NaiveDateTime;

use common_domain::ids::OrganizationId;
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Queryable, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::organization)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OrganizationRow {
    pub id: OrganizationId,
    pub trade_name: String,
    pub slug: String,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub invite_link_hash: Option<String>,
    pub default_country: String,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::organization)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OrganizationRowNew {
    pub id: OrganizationId,
    pub slug: String,
    pub trade_name: String,
    pub default_country: String,
}
