//! Keyset pages of entitlement rows for the APIs: an owner's open rows, a row's versions.
use crate::domain::entitlements::Entitlement;
use crate::{Store, StoreResult};
use common_domain::ids::{EntitlementEntityId, EntitlementId, FeatureId, TenantId};
use diesel_models::entitlements::EntitlementRow;

fn sql_limit(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

impl Store {
    /// The owner's open rows (of `feature` only, when given) after the row `after`, by id.
    pub async fn list_open_entitlements_after(
        &self,
        tenant_id: TenantId,
        owner: EntitlementEntityId,
        feature: Option<FeatureId>,
        after: Option<EntitlementId>,
        limit: usize,
    ) -> StoreResult<Vec<Entitlement>> {
        let mut conn = self.get_conn().await?;
        EntitlementRow::page_open_of_owner(
            &mut conn,
            tenant_id,
            owner,
            feature,
            after,
            sql_limit(limit),
        )
        .await?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    /// The row's versions before the version `before`, newest first.
    pub async fn entitlement_history_before(
        &self,
        tenant_id: TenantId,
        id: EntitlementId,
        before: Option<EntitlementId>,
        limit: usize,
    ) -> StoreResult<Vec<Entitlement>> {
        let mut conn = self.get_conn().await?;
        EntitlementRow::page_history(&mut conn, tenant_id, id, before, sql_limit(limit))
            .await?
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }
}
