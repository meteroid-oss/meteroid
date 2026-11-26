//! Keyset reads of entitlement rows: an owner's open rows by lineage id, a lineage's versions
//! newest first. Rows of archived features stay hidden, as everywhere else.
use crate::entitlements::EntitlementRow;
use crate::enums::{EntitlementEntityTypeEnum, FeatureStatusEnum};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::{EntitlementEntityId, EntitlementId, FeatureId, TenantId};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

#[diesel::dsl::auto_type(no_type_alias)]
fn archived_feature_ids(param_tenant_id: TenantId) -> _ {
    let archived: FeatureStatusEnum = FeatureStatusEnum::Archived;
    crate::schema::feature::table
        .filter(crate::schema::feature::tenant_id.eq(param_tenant_id))
        .filter(crate::schema::feature::status.eq(archived))
        .select(crate::schema::feature::id)
}

impl EntitlementRow {
    /// The owner's open rows (of `feature` only, when given) after the lineage `after`, by lineage.
    pub async fn page_open_of_owner(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        owner: EntitlementEntityId,
        feature: Option<FeatureId>,
        after: Option<EntitlementId>,
        limit: i64,
    ) -> DbResult<Vec<EntitlementRow>> {
        use crate::schema::entitlement::dsl;

        let owner_type: EntitlementEntityTypeEnum = (&owner).into();
        let mut query = dsl::entitlement
            .filter(dsl::tenant_id.eq(param_tenant_id))
            .filter(dsl::entity_type.eq(owner_type))
            .filter(dsl::entity_id.eq(owner.as_uuid()))
            .filter(dsl::revoked_at.is_null())
            .filter(dsl::feature_id.ne_all(archived_feature_ids(param_tenant_id)))
            .into_boxed();
        if let Some(feature) = feature {
            query = query.filter(dsl::feature_id.eq(feature));
        }
        if let Some(after) = after {
            query = query.filter(dsl::lineage_id.gt(after));
        }
        query
            .order(dsl::lineage_id.asc())
            .limit(limit)
            .select(EntitlementRow::as_select())
            .get_results(conn)
            .await
            .attach("Error while paging an owner's entitlements")
            .into_db_result()
    }

    /// The lineage's versions before the version `before`, newest first (version ids are
    /// UUIDv7: their creation order).
    pub async fn page_history(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        lineage: EntitlementId,
        before: Option<EntitlementId>,
        limit: i64,
    ) -> DbResult<Vec<EntitlementRow>> {
        use crate::schema::entitlement::dsl;

        let mut query = dsl::entitlement
            .filter(dsl::tenant_id.eq(param_tenant_id))
            .filter(dsl::lineage_id.eq(lineage))
            .filter(dsl::feature_id.ne_all(archived_feature_ids(param_tenant_id)))
            .into_boxed();
        if let Some(before) = before {
            query = query.filter(dsl::id.lt(before));
        }
        query
            .order(dsl::id.desc())
            .limit(limit)
            .select(EntitlementRow::as_select())
            .get_results(conn)
            .await
            .attach("Error while paging an entitlement's history")
            .into_db_result()
    }
}
