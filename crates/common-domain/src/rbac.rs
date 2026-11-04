//! RBAC capability vocabulary.
//!
//! `Permission` is the compile-time source of truth for what a role can do.
//! The code owns the vocabulary: a permission that no route requires is inert,
//! and a route can only require a permission that exists here. Roles (built-in
//! or custom) are named *sets* of these permissions, resolved to
//! [`RolePermissions`] at request time.

use crate::auth::OrgMemberRole;
use serde::{Deserialize, Serialize};

/// A single capability. Grouped by product area, split read vs. manage where a
/// read-only role is meaningful. Stored and transmitted as the stable
/// [`Permission::code`] string — never the discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    CustomersRead,
    CustomersManage,
    // Billing is split per sub-area so a role can e.g. manage subscriptions
    // without touching invoices. Codes keep the `billing:` prefix so they
    // group together in the UI.
    SubscriptionsRead,
    SubscriptionsManage,
    // Invoices and credit notes (credit notes are invoice corrections).
    InvoicesRead,
    InvoicesManage,
    QuotesRead,
    QuotesManage,
    // Plans, prices, products, add-ons, coupons, schedules, metrics, entitlements.
    CatalogRead,
    CatalogManage,
    // Stats, segments, activity feed, saved views.
    AnalyticsRead,
    // Raw usage/metering events explorer (EventsService). Read-only in-app:
    // ingestion and CSV import go through the metering pipeline and batch jobs,
    // not this capability. Kept separate from settings so a role can inspect
    // usage events without seeing connectors, webhooks or API tokens.
    EventsRead,
    AlertsRead,
    AlertsManage,
    // Targets and the plan: reading them, and setting them. Apart from alerts, so a role that
    // runs monitors does not rewrite the plan the board reads.
    TargetsRead,
    TargetsManage,
    // Invoicing entities, bank accounts, taxes, tenants, connectors, webhooks,
    // notification channels, api tokens, events, batch jobs.
    SettingsRead,
    SettingsManage,
    // Invite/remove members, assign roles.
    MembersManage,
    // Create/edit/delete custom roles.
    RolesManage,
    // Org settings, MFA policy, security & audit configuration.
    OrganizationManage,
}

impl Permission {
    pub const ALL: [Permission; 21] = [
        Permission::CustomersRead,
        Permission::CustomersManage,
        Permission::SubscriptionsRead,
        Permission::SubscriptionsManage,
        Permission::InvoicesRead,
        Permission::InvoicesManage,
        Permission::QuotesRead,
        Permission::QuotesManage,
        Permission::CatalogRead,
        Permission::CatalogManage,
        Permission::AnalyticsRead,
        Permission::EventsRead,
        Permission::AlertsRead,
        Permission::AlertsManage,
        Permission::TargetsRead,
        Permission::TargetsManage,
        Permission::SettingsRead,
        Permission::SettingsManage,
        Permission::MembersManage,
        Permission::RolesManage,
        Permission::OrganizationManage,
    ];

    /// Governance capabilities that a plain `Member` never holds. Everything
    /// else is granted to members to preserve pre-RBAC behaviour.
    pub const GOVERNANCE: [Permission; 3] = [
        Permission::MembersManage,
        Permission::RolesManage,
        Permission::OrganizationManage,
    ];

    /// Stable wire/storage identifier. Changing these is a breaking change to
    /// persisted custom roles.
    pub const fn code(self) -> &'static str {
        match self {
            Permission::CustomersRead => "customers:read",
            Permission::CustomersManage => "customers:manage",
            Permission::SubscriptionsRead => "billing:subscriptions:read",
            Permission::SubscriptionsManage => "billing:subscriptions:manage",
            Permission::InvoicesRead => "billing:invoices:read",
            Permission::InvoicesManage => "billing:invoices:manage",
            Permission::QuotesRead => "billing:quotes:read",
            Permission::QuotesManage => "billing:quotes:manage",
            Permission::CatalogRead => "catalog:read",
            Permission::CatalogManage => "catalog:manage",
            Permission::AnalyticsRead => "analytics:read",
            Permission::EventsRead => "events:read",
            Permission::AlertsRead => "alerts:read",
            Permission::AlertsManage => "alerts:manage",
            Permission::TargetsRead => "targets:read",
            Permission::TargetsManage => "targets:manage",
            Permission::SettingsRead => "settings:read",
            Permission::SettingsManage => "settings:manage",
            Permission::MembersManage => "members:manage",
            Permission::RolesManage => "roles:manage",
            Permission::OrganizationManage => "organization:manage",
        }
    }

    pub fn from_code(code: &str) -> Option<Permission> {
        Permission::ALL.into_iter().find(|p| p.code() == code)
    }

    /// Parse a *stored* code, also accepting the pre-split umbrella codes
    /// (`billing:read`/`billing:manage`) which expand to every billing
    /// sub-area. A migration rewrites stored rows; this covers rows written
    /// before it ran so a role never silently loses billing access. API input
    /// goes through the strict [`Permission::from_code`] instead.
    pub fn expand_stored_code(code: &str) -> Vec<Permission> {
        match code {
            "billing:read" => vec![
                Permission::SubscriptionsRead,
                Permission::InvoicesRead,
                Permission::QuotesRead,
            ],
            "billing:manage" => vec![
                Permission::SubscriptionsManage,
                Permission::InvoicesManage,
                Permission::QuotesManage,
            ],
            _ => Permission::from_code(code).into_iter().collect(),
        }
    }

    /// The manage capability that subsumes this read capability, if any. Manage
    /// is a superset of view, so holding the returned permission implies `self`.
    /// `None` for permissions with no read/manage split (e.g. `AnalyticsRead`)
    /// or that are themselves a manage capability.
    pub const fn implied_by_manage(self) -> Option<Permission> {
        match self {
            Permission::CustomersRead => Some(Permission::CustomersManage),
            Permission::SubscriptionsRead => Some(Permission::SubscriptionsManage),
            Permission::InvoicesRead => Some(Permission::InvoicesManage),
            Permission::QuotesRead => Some(Permission::QuotesManage),
            Permission::CatalogRead => Some(Permission::CatalogManage),
            Permission::AlertsRead => Some(Permission::AlertsManage),
            Permission::TargetsRead => Some(Permission::TargetsManage),
            Permission::SettingsRead => Some(Permission::SettingsManage),
            _ => None,
        }
    }
}

// ALL must list every variant; bump this when adding a permission.
const _: () = assert!(Permission::ALL.len() == 21);

/// A resolved set of capabilities for an actor in an organization.
///
/// `All` is the admin wildcard: it holds every permission, including ones added
/// after the role was created. This is why admin is a variant rather than a
/// stored snapshot of [`Permission::ALL`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RolePermissions {
    All,
    Set(Vec<Permission>),
}

/// The permissions the auth middleware resolved for a member (or a support grant), carried in
/// the request extensions so handlers scope data by the same answer the route was allowed on.
#[derive(Debug, Clone)]
pub struct ResolvedPermissions(pub RolePermissions);

impl RolePermissions {
    pub fn contains(&self, permission: Permission) -> bool {
        match self {
            RolePermissions::All => true,
            RolePermissions::Set(set) => {
                set.contains(&permission)
                    || permission
                        .implied_by_manage()
                        .is_some_and(|manage| set.contains(&manage))
            }
        }
    }

    /// The capabilities of a built-in `Member`: everything except governance.
    pub fn member_default() -> RolePermissions {
        RolePermissions::Set(
            Permission::ALL
                .into_iter()
                .filter(|p| !Permission::GOVERNANCE.contains(p))
                .collect(),
        )
    }

    pub fn none() -> RolePermissions {
        RolePermissions::Set(vec![])
    }

    /// Read-only capabilities: every `*Read` permission and nothing that mutates.
    /// Used for platform-admin support sessions, so `enforce` allows every getter
    /// and denies every manage/mutating route.
    pub fn read_only() -> RolePermissions {
        RolePermissions::Set(
            Permission::ALL
                .into_iter()
                .filter(|p| p.code().ends_with(":read"))
                .collect(),
        )
    }

    /// Resolve the built-in permissions for an [`OrgMemberRole`]. Custom roles
    /// bypass this and use their stored [`Permission`] set directly.
    pub fn for_builtin_role(role: OrgMemberRole) -> RolePermissions {
        match role {
            OrgMemberRole::Admin => RolePermissions::All,
            OrgMemberRole::Member => RolePermissions::member_default(),
        }
    }

    /// Parse a stored set of permission codes. Legacy umbrella codes expand
    /// via [`Permission::expand_stored_code`]; any other code no longer known
    /// to the current binary (removed/renamed) is silently dropped.
    pub fn from_codes<I, S>(codes: I) -> RolePermissions
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        RolePermissions::Set(
            codes
                .into_iter()
                .flat_map(|c| Permission::expand_stored_code(c.as_ref()))
                .collect(),
        )
    }

    pub fn to_codes(&self) -> Vec<String> {
        match self {
            RolePermissions::All => Permission::ALL
                .iter()
                .map(|p| p.code().to_string())
                .collect(),
            RolePermissions::Set(set) => set.iter().map(|p| p.code().to_string()).collect(),
        }
    }

    /// Codes for the client, expanded so a held `manage` also lists its `read`
    /// (mirrors [`RolePermissions::contains`]'s implication rule). Keeps the
    /// frontend's `has("billing:invoices:read")` true when only
    /// `billing:invoices:manage` was stored, so one rule governs both
    /// enforcement and UI gating.
    pub fn to_effective_codes(&self) -> Vec<String> {
        match self {
            RolePermissions::All => self.to_codes(),
            RolePermissions::Set(set) => {
                let mut perms = set.clone();
                for read in Permission::ALL {
                    if let Some(manage) = read.implied_by_manage()
                        && set.contains(&manage)
                        && !perms.contains(&read)
                    {
                        perms.push(read);
                    }
                }
                perms.iter().map(|p| p.code().to_string()).collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_roundtrip() {
        for p in Permission::ALL {
            assert_eq!(Permission::from_code(p.code()), Some(p));
        }
        let unique: std::collections::HashSet<_> =
            Permission::ALL.iter().map(|p| p.code()).collect();
        assert_eq!(unique.len(), Permission::ALL.len());
    }

    #[test]
    fn admin_holds_everything_member_excludes_governance() {
        let admin = RolePermissions::for_builtin_role(OrgMemberRole::Admin);
        let member = RolePermissions::for_builtin_role(OrgMemberRole::Member);
        for p in Permission::ALL {
            assert!(admin.contains(p));
            assert_eq!(member.contains(p), !Permission::GOVERNANCE.contains(&p));
        }
    }

    #[test]
    fn manage_implies_read() {
        // A role granted only the manage capability can still perform reads.
        let set = RolePermissions::from_codes(["catalog:manage"]);
        assert!(set.contains(Permission::CatalogManage));
        assert!(set.contains(Permission::CatalogRead));
        // The implication does not leak across product areas.
        assert!(!set.contains(Permission::InvoicesRead));
        // Read without manage confers no manage capability.
        let read_only = RolePermissions::from_codes(["catalog:read"]);
        assert!(read_only.contains(Permission::CatalogRead));
        assert!(!read_only.contains(Permission::CatalogManage));
        // Every read with a manage counterpart is implied by that manage.
        for p in Permission::ALL {
            if let Some(manage) = p.implied_by_manage() {
                assert!(RolePermissions::Set(vec![manage]).contains(p));
            }
        }
    }

    #[test]
    fn effective_codes_expand_manage_to_read() {
        // A manage-only custom role reports its implied read to the client, so
        // `has("catalog:read")` matches server-side `contains`.
        let set = RolePermissions::from_codes(["catalog:manage"]);
        let codes = set.to_effective_codes();
        assert!(codes.contains(&"catalog:manage".to_string()));
        assert!(codes.contains(&"catalog:read".to_string()));
        assert!(!codes.contains(&"billing:invoices:read".to_string()));
        // Admin still lists the full permission set.
        assert_eq!(
            RolePermissions::All.to_effective_codes().len(),
            Permission::ALL.len()
        );
    }

    #[test]
    fn from_codes_drops_unknown() {
        let set = RolePermissions::from_codes(["customers:read", "bogus:code"]);
        assert!(set.contains(Permission::CustomersRead));
        assert_eq!(set, RolePermissions::Set(vec![Permission::CustomersRead]));
    }

    #[test]
    fn billing_areas_are_independent() {
        // The user-facing promise of the split: managing subscriptions grants
        // nothing on invoices or quotes, and vice versa.
        let subs = RolePermissions::from_codes(["billing:subscriptions:manage"]);
        assert!(subs.contains(Permission::SubscriptionsManage));
        assert!(subs.contains(Permission::SubscriptionsRead));
        assert!(!subs.contains(Permission::InvoicesRead));
        assert!(!subs.contains(Permission::QuotesRead));
        let invoices = RolePermissions::from_codes(["billing:invoices:manage"]);
        assert!(invoices.contains(Permission::InvoicesManage));
        assert!(!invoices.contains(Permission::SubscriptionsRead));
    }

    #[test]
    fn legacy_umbrella_codes_expand_to_all_billing_areas() {
        // Roles stored before the split keep their full billing access.
        let read = RolePermissions::from_codes(["billing:read"]);
        assert!(read.contains(Permission::SubscriptionsRead));
        assert!(read.contains(Permission::InvoicesRead));
        assert!(read.contains(Permission::QuotesRead));
        assert!(!read.contains(Permission::SubscriptionsManage));
        let manage = RolePermissions::from_codes(["billing:manage"]);
        for p in [
            Permission::SubscriptionsManage,
            Permission::InvoicesManage,
            Permission::QuotesManage,
            Permission::SubscriptionsRead,
        ] {
            assert!(manage.contains(p));
        }
        // Strict API-input parsing does not accept the legacy codes.
        assert_eq!(Permission::from_code("billing:manage"), None);
    }

    #[test]
    fn events_read_is_independent_read_only_capability() {
        // Reading usage events is its own capability, not implied by settings.
        let settings = RolePermissions::from_codes(["settings:manage"]);
        assert!(!settings.contains(Permission::EventsRead));
        let events = RolePermissions::from_codes(["events:read"]);
        assert!(events.contains(Permission::EventsRead));
        assert!(!events.contains(Permission::SettingsRead));
        // Read-only: no manage counterpart, yet held by Member and support sessions.
        assert_eq!(Permission::EventsRead.implied_by_manage(), None);
        assert!(RolePermissions::member_default().contains(Permission::EventsRead));
        assert!(RolePermissions::read_only().contains(Permission::EventsRead));
    }

    #[test]
    fn targets_are_apart_from_alerts() {
        // Running monitors does not grant rewriting the plan, nor reading it.
        let alerts = RolePermissions::from_codes(["alerts:manage"]);
        assert!(!alerts.contains(Permission::TargetsRead));
        assert!(!alerts.contains(Permission::TargetsManage));
        let reader = RolePermissions::from_codes(["targets:read"]);
        assert!(reader.contains(Permission::TargetsRead));
        assert!(!reader.contains(Permission::TargetsManage));
        assert!(RolePermissions::from_codes(["targets:manage"]).contains(Permission::TargetsRead));
    }

    #[test]
    fn read_only_grants_reads_and_denies_mutations() {
        let ro = RolePermissions::read_only();
        for p in Permission::ALL {
            // Security-critical: a support session must never hold a mutating capability.
            assert_eq!(ro.contains(p), p.code().ends_with(":read"), "{:?}", p);
        }
        assert!(ro.contains(Permission::CustomersRead));
        assert!(!ro.contains(Permission::CustomersManage));
        assert!(!ro.contains(Permission::OrganizationManage));
    }
}
