use crate::domain::enums::TenantEnvironmentEnum;
use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, OrganizationId, TenantId};
use diesel_models::tenants::{TenantRow, TenantRowNew, TenantRowPatch};
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
}

#[derive(Clone, Debug)]
pub struct FullTenantNew {
    pub name: String,
    pub slug: String,
    pub organization_id: OrganizationId,
    pub reporting_currency: String,
    pub environment: TenantEnvironmentEnum,
}
impl From<FullTenantNew> for TenantRowNew {
    fn from(val: FullTenantNew) -> Self {
        TenantRowNew {
            name: val.name,
            slug: val.slug,
            organization_id: val.organization_id,
            reporting_currency: val.reporting_currency.clone(),
            id: TenantId::new(),
            available_currencies: vec![Some(val.reporting_currency)],
            environment: val.environment.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TenantNew {
    pub name: String,
    pub environment: TenantEnvironmentEnum,
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
}
