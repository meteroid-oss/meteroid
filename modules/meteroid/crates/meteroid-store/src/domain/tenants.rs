use chrono::NaiveDateTime;
use o2o::o2o;
use uuid::Uuid;

use crate::domain::enums::TenantEnvironmentEnum;
use diesel_models::tenants::TenantRow;
use diesel_models::tenants::TenantRowNew;

#[derive(Clone, Debug, o2o)]
#[from_owned(TenantRow)]
#[owned_into(TenantRow)]
pub struct Tenant {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub organization_id: Uuid,
    pub currency: String,
    #[map(~.into())]
    pub environment: TenantEnvironmentEnum,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(TenantRowNew)]
#[ghosts(id: {uuid::Uuid::now_v7()})]
pub struct TenantNew {
    pub name: String,
    pub slug: String,
    pub organization_id: Uuid,
    pub currency: String,
    #[map(~.into())]
    pub environment: TenantEnvironmentEnum,
}
