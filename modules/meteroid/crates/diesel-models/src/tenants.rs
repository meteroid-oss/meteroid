use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::TenantEnvironmentEnum;

use diesel::{Identifiable, Insertable, Queryable, Selectable};

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
    pub environment: Option<TenantEnvironmentEnum>,
}
