use chrono::{DateTime, Utc};
use common_domain::ids::{OrganizationId, RoleId};
use common_domain::rbac::{Permission, RolePermissions};

use crate::domain::enums::OrganizationUserRole;
use diesel_models::roles::RoleRow;

/// A custom, organization-scoped role: a named set of permissions. Built-in
/// Admin/Member are not stored here — they live on `organization_member.role`.
#[derive(Debug, Clone)]
pub struct Role {
    pub id: RoleId,
    pub organization_id: OrganizationId,
    pub name: String,
    pub permissions: Vec<Permission>,
    pub is_system: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<RoleRow> for Role {
    fn from(row: RoleRow) -> Self {
        Role {
            id: row.id,
            organization_id: row.organization_id,
            name: row.name,
            // Stored-code parsing: expands legacy umbrella codes (pre-split
            // `billing:*`) and drops codes unknown to this binary.
            permissions: row
                .permissions
                .into_iter()
                .flatten()
                .flat_map(|c| Permission::expand_stored_code(&c))
                .collect(),
            is_system: row.is_system,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A member's effective access in an organization: their built-in tier plus the
/// resolved permission set (Admin wildcard, custom-role set, or Member default).
#[derive(Debug, Clone)]
pub struct MemberAccess {
    pub role: OrganizationUserRole,
    pub permissions: RolePermissions,
}
