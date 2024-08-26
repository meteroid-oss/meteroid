use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::TenantEnvironmentEnum;

use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Clone, Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::tenant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TenantRow {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub organization_id: Uuid,
    // the reporting currency, used in dashboards
    pub currency: String,
    pub environment: TenantEnvironmentEnum,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::tenant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TenantRowNew {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub organization_id: Uuid,
    pub currency: String,
    pub environment: TenantEnvironmentEnum,
}


#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::tenant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TenantRowPatch {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub currency: Option<String>,
    pub environment: Option<TenantEnvironmentEnum>,
}
