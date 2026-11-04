use common_domain::rbac::{Permission, RolePermissions};
use meteroid_grpc::meteroid::api::rbac::v1 as server;
use meteroid_store::domain::roles::Role;

/// Stable identifiers for the built-in tiers. They are not `RoleId`s — the
/// built-ins have no `role` row; they are surfaced for display only and are
/// assigned via the `OrganizationUserRole` enum, not `custom_role_id`.
pub const BUILTIN_ADMIN_ID: &str = "system_admin";
pub const BUILTIN_MEMBER_ID: &str = "system_member";

pub fn is_builtin_role_id(id: &str) -> bool {
    id == BUILTIN_ADMIN_ID || id == BUILTIN_MEMBER_ID
}

/// Built-in tiers rendered as read-only roles, derived from code so they can
/// never drift from the enforced permission sets.
pub fn builtin_roles() -> Vec<server::CustomRole> {
    vec![
        builtin_role(BUILTIN_ADMIN_ID, "Admin", RolePermissions::All),
        builtin_role(
            BUILTIN_MEMBER_ID,
            "Member",
            RolePermissions::member_default(),
        ),
    ]
}

fn builtin_role(id: &str, name: &str, permissions: RolePermissions) -> server::CustomRole {
    server::CustomRole {
        id: id.to_string(),
        name: name.to_string(),
        permission_codes: permissions.to_codes(),
        is_system: true,
    }
}

pub fn permission_to_proto(permission: Permission) -> server::Permission {
    let code = permission.code();
    server::Permission {
        code: code.to_string(),
        group: code.split(':').next().unwrap_or(code).to_string(),
    }
}

pub fn role_to_proto(role: Role) -> server::CustomRole {
    server::CustomRole {
        id: role.id.as_proto(),
        name: role.name,
        permission_codes: role
            .permissions
            .iter()
            .map(|p| p.code().to_string())
            .collect(),
        is_system: role.is_system,
    }
}
