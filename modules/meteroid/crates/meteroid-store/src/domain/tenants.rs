use crate::domain::Organization;
use crate::domain::enums::TenantEnvironmentEnum;
use chrono::NaiveDateTime;
use common_domain::ids::{OrganizationId, TenantId};
use diesel_models::tenants::{TenantRow, TenantRowPatch, TenantWithOrganizationRow};
use o2o::o2o;

#[derive(Clone, Debug, o2o)]
#[from_owned(TenantRow)]
#[owned_into(TenantRow)]
pub struct Tenant {
    pub id: TenantId,
    pub name: String,
    pub slug: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub organization_id: OrganizationId,
    pub reporting_currency: String,
    #[map(~.into())]
    pub environment: TenantEnvironmentEnum,
    pub available_currencies: Vec<Option<String>>,
    pub disable_emails: bool,
}

#[derive(Clone, Debug)]
pub struct TenantNew {
    pub name: String,
    pub environment: TenantEnvironmentEnum,
    pub disable_emails: Option<bool>,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(TenantRowPatch)]
pub struct TenantUpdate {
    #[ghost({None})]
    pub trade_name: Option<String>,
    pub name: Option<String>,
    pub slug: Option<String>,
    #[map(~.map(| x | x.into()))]
    pub environment: Option<TenantEnvironmentEnum>,
    pub reporting_currency: Option<String>,
    pub disable_emails: Option<bool>,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(TenantWithOrganizationRow)]
pub struct TenantWithOrganization {
    #[map(~.into())]
    pub tenant: Tenant,
    #[map(~.into())]
    pub organization: Organization,
}
