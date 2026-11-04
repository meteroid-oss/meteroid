use std::sync::Arc;

use common_domain::ids::{OrganizationId, UserId};
use common_domain::rbac::RolePermissions;
use meteroid_store::domain::enums::OrganizationUserRole;
use meteroid_store::domain::roles::MemberAccess;

/// Access level of a support-access grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportGrantMode {
    ReadOnly,
    /// Member-level: everything except org governance.
    ReadWrite,
    /// Org-admin-level, governance included (members, roles, SSO, security log).
    Admin,
}

impl SupportGrantMode {
    pub fn member_access(self) -> MemberAccess {
        let (role, permissions) = match self {
            SupportGrantMode::ReadOnly => {
                (OrganizationUserRole::Member, RolePermissions::read_only())
            }
            SupportGrantMode::ReadWrite => (
                OrganizationUserRole::Member,
                RolePermissions::member_default(),
            ),
            SupportGrantMode::Admin => (OrganizationUserRole::Admin, RolePermissions::All),
        };
        MemberAccess { role, permissions }
    }
}

/// Reads platform-admin "support access" grants. Implemented in the application
/// crate (Redis-backed); kept as a trait here so this middleware stays decoupled
/// from Redis. A grant lets a platform admin reach an organization they are not a
/// member of, for a bounded time, at the granted access level.
#[async_trait::async_trait]
pub trait SupportGrantReader: Send + Sync {
    /// The access level of a live, authentic grant for this (admin user, org), or
    /// `None` if there is none. Implementations must fail closed (return `None`)
    /// on any backend error or if the grant fails signature/expiry verification.
    async fn grant_mode(&self, user_id: UserId, org_id: OrganizationId)
    -> Option<SupportGrantMode>;
}

pub type SharedSupportGrantReader = Arc<dyn SupportGrantReader>;
