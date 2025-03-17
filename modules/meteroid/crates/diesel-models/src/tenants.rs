use chrono::NaiveDateTime;

use crate::enums::TenantEnvironmentEnum;

use crate::organizations::OrganizationRow;
use common_domain::ids::{OrganizationId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Clone, Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::tenant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TenantRow {
    pub id: TenantId,
    pub name: String,
    pub slug: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub organization_id: OrganizationId,
    // the reporting currency, used in dashboards
    pub reporting_currency: String,
    pub environment: TenantEnvironmentEnum,
    pub available_currencies: Vec<Option<String>>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::tenant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TenantRowNew {
    pub id: TenantId,
    pub name: String,
    pub slug: String,
    pub organization_id: OrganizationId,
    pub reporting_currency: String,
    pub environment: TenantEnvironmentEnum,
    pub available_currencies: Vec<Option<String>>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::tenant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TenantRowPatch {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub reporting_currency: Option<String>,
    pub environment: Option<TenantEnvironmentEnum>,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TenantWithOrganizationRow {
    #[diesel(embed)]
    pub tenant: TenantRow,
    #[diesel(embed)]
    pub organization: OrganizationRow,
}
