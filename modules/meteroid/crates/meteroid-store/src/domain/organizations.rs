use chrono::NaiveDateTime;
use o2o::o2o;
use uuid::Uuid;

use diesel_models::organizations::OrganizationRow;
use diesel_models::organizations::OrganizationRowNew;

#[derive(Clone, Debug, o2o)]
#[from_owned(OrganizationRow)]
#[owned_into(OrganizationRow)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub invite_link_hash: Option<String>,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(OrganizationRowNew)]
pub struct OrganizationNew {
    pub name: String,
    pub slug: String,
}
