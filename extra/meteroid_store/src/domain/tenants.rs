use chrono::NaiveDateTime;
use o2o::o2o;
use uuid::Uuid;

use diesel_models::tenants::Tenant as DieselTenant;
use diesel_models::tenants::TenantNew as DieselTenantNew;

#[derive(Clone, Debug, o2o)]
#[from_owned(DieselTenant)]
#[owned_into(DieselTenant)]
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

#[derive(Clone, Debug, o2o)]
#[owned_into(DieselTenantNew)]
#[ghosts(id: {uuid::Uuid::now_v7()})]
pub struct TenantNew {
    pub name: String,
    pub slug: String,
    pub organization_id: Uuid,
    pub currency: String,
}
