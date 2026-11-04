use chrono::{DateTime, Utc};
use common_domain::ids::{OrganizationId, RoleId};
use diesel::{Identifiable, Queryable, Selectable};

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::role)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RoleRow {
    pub id: RoleId,
    pub organization_id: OrganizationId,
    pub name: String,
    // Postgres text[] elements are nullable, hence Option.
    pub permissions: Vec<Option<String>>,
    pub is_system: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
