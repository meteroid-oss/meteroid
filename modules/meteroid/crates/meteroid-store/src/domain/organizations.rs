use chrono::NaiveDateTime;
use o2o::o2o;
use uuid::Uuid;

#[derive(Clone, Debug, o2o)]
#[from_owned(diesel_models::organizations::Organization)]
#[owned_into(diesel_models::organizations::Organization)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
    pub invite_link_hash: Option<String>,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(diesel_models::organizations::OrganizationNew)]
pub struct OrganizationNew {
    pub name: String,
    pub slug: String,
}
