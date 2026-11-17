//! The release of billing from facts, held to an allowlist of tenants while it rolls out
//! (`BILLING_CONTROLLER_RELEASE=allowlist`): a held tenant's billing writes no document, no
//! payment and no outbox event until a `billing_controller_release` row names it (or names no
//! tenant, releasing all). Its requests write their facts and state, their documents left to
//! the controller. The default, `all`, holds nothing.
use std::str::FromStr;

use common_domain::ids::TenantId;
use diesel_models::billing_controller::BillingControllerRow;
use error_stack::Report;

use crate::StoreResult;
use crate::errors::StoreError;
use crate::services::Services;
use crate::store::PgConn;

/// `BILLING_CONTROLLER_RELEASE`: which tenants the billing controller bills.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BillingRelease {
    /// Every tenant.
    #[default]
    All,
    /// Only the tenants a `billing_controller_release` row names, all of them once a row names none.
    Allowlist,
}

impl BillingRelease {
    /// Whether the release is held to the allowlist.
    pub fn held_to_allowlist(self) -> bool {
        self == BillingRelease::Allowlist
    }
}

impl FromStr for BillingRelease {
    type Err = String;

    /// Blank means unset, as `BILLING_CONTROLLER_RELEASE=` in a compose file.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "" | "all" => Ok(BillingRelease::All),
            "allowlist" => Ok(BillingRelease::Allowlist),
            other => Err(format!(
                "BILLING_CONTROLLER_RELEASE is `{other}`, expected `all` or `allowlist`"
            )),
        }
    }
}

/// A held tenant's change dated `on`, its facts written: its documents left to the controller,
/// which settles them on release (R-120).
pub(crate) async fn leave_to_controller(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: common_domain::ids::SubscriptionId,
    on: chrono::NaiveDate,
    kind: diesel_models::enums::SubscriptionChangeKindEnum,
) -> StoreResult<()> {
    diesel_models::subscription_changes::SubscriptionChangeRow::mark_settlement_due(
        conn,
        tenant_id,
        subscription_id,
        on,
        kind,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)
}

impl Services {
    /// Whether the tenant's documents wait for its release.
    pub(crate) async fn is_held(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> StoreResult<bool> {
        if !self.store.settings.billing_release.held_to_allowlist() {
            return Ok(false);
        }
        BillingControllerRow::is_held(conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }
}

#[cfg(test)]
mod tests {
    use super::BillingRelease;

    #[test]
    fn the_release_setting_reads_all_or_allowlist_and_refuses_anything_else() {
        assert_eq!("all".parse(), Ok(BillingRelease::All));
        assert_eq!("".parse(), Ok(BillingRelease::All));
        assert_eq!("allowlist".parse(), Ok(BillingRelease::Allowlist));
        assert!("allow-list".parse::<BillingRelease>().is_err());
        assert!("ALL".parse::<BillingRelease>().is_err());
    }
}
