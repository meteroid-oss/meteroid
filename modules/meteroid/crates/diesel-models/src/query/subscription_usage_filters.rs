use crate::errors::IntoDbResult;
use crate::subscription_usage_filters::{
    SubscriptionUsageFilterRow, SubscriptionUsageFilterRowNew,
};
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use common_domain::ids::{SubscriptionId, TenantId};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

impl SubscriptionUsageFilterRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::subscription_usage_filter::dsl as f;

        let query = diesel::insert_into(f::subscription_usage_filter).values(self);
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while inserting a usage filter version")
            .into_db_result()
    }
}

impl SubscriptionUsageFilterRow {
    /// The versions of the subscriptions, oldest first.
    pub async fn list_by_subscriptions(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_ids: &[SubscriptionId],
    ) -> DbResult<Vec<SubscriptionUsageFilterRow>> {
        use crate::schema::subscription_usage_filter::dsl as f;

        if subscription_ids.is_empty() {
            return Ok(Vec::new());
        }
        let query = f::subscription_usage_filter
            .filter(f::tenant_id.eq(tenant_id))
            .filter(f::subscription_id.eq_any(subscription_ids))
            .order((f::subscription_id, f::effective_from))
            .select(SubscriptionUsageFilterRow::as_select());
        crate::debug_sql(&query);
        query
            .get_results(conn)
            .await
            .attach("Error while listing usage filter versions")
            .into_db_result()
    }

    /// Ends a version at `until`.
    pub async fn close(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: Uuid,
        until: NaiveDateTime,
    ) -> DbResult<()> {
        use crate::schema::subscription_usage_filter::dsl as f;

        let query = diesel::update(
            f::subscription_usage_filter
                .filter(f::id.eq(id))
                .filter(f::tenant_id.eq(tenant_id)),
        )
        .set(f::effective_until.eq(Some(until)));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while closing a usage filter version")
            .into_db_result()
    }

    /// Replaces the filter of a version that starts where the new one would.
    pub async fn set_filter(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: Uuid,
        property: Option<String>,
        value: Option<String>,
    ) -> DbResult<()> {
        use crate::schema::subscription_usage_filter::dsl as f;

        let query = diesel::update(
            f::subscription_usage_filter
                .filter(f::id.eq(id))
                .filter(f::tenant_id.eq(tenant_id)),
        )
        .set((f::property.eq(property), f::value.eq(value)));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while replacing a usage filter version")
            .into_db_result()
    }
}
