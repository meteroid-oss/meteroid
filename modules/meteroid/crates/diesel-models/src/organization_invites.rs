use chrono::DateTime;
use chrono::Utc;
use common_domain::ids::{OrganizationId, OrganizationInviteId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

use crate::enums::OrganizationUserRole;

#[derive(Debug, Queryable, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::organization_invite)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OrganizationInviteRow {
    pub id: OrganizationInviteId,
    pub organization_id: OrganizationId,
    pub invited_email: String,
    pub invited_by: Uuid,
    pub role: OrganizationUserRole,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::organization_invite)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OrganizationInviteRowNew {
    pub id: OrganizationInviteId,
    pub organization_id: OrganizationId,
    pub invited_email: String,
    pub invited_by: Uuid,
    pub role: OrganizationUserRole,
    pub expires_at: DateTime<Utc>,
}
