use crate::errors::IntoDbResult;
use crate::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use crate::{DbResult, PgConn};
use chrono::NaiveDate;
use common_domain::ids::{ProductId, SubscriptionAddOnId, SubscriptionId, TenantId};
use diesel::QueryDsl;
use diesel::{BoolExpressionMethods, ExpressionMethods, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl SubscriptionAddOnRow {
    /// Ends, on `effective_to`, the active add-ons whose add_on_id is not in the allowed set:
    /// their history stays, so the period's invoice still bills what they owe up to then.
    pub async fn close_incompatible(
        conn: &mut PgConn,
        subscription_id: &SubscriptionId,
        compatible_add_on_ids: &[common_domain::ids::AddOnId],
        effective_to: NaiveDate,
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        let query = diesel::update(
            sao_dsl::subscription_add_on
                .filter(sao_dsl::subscription_id.eq(subscription_id))
                .filter(sao_dsl::effective_to.is_null())
                .filter(diesel::dsl::not(
                    sao_dsl::add_on_id.eq_any(compatible_add_on_ids),
                )),
        )
        .set(sao_dsl::effective_to.eq(effective_to))
        .returning(SubscriptionAddOnRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while closing incompatible subscription add-ons")
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        batch: Vec<&SubscriptionAddOnRowNew>,
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        let query = diesel::insert_into(sao_dsl::subscription_add_on).values(
            batch
                .iter()
                .map(|row| {
                    (
                        *row,
                        crate::schema::subscription_add_on::created_at
                            .eq(common_domain::clock::now().naive_utc()),
                    )
                })
                .collect::<Vec<_>>(),
        );

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while inserting SubscriptionAddOn batch")
            .into_db_result()
    }

    /// Fetch the lineage root for a set of add-on ids. Returns `(id, lineage_id)`
    /// pairs; a `None` lineage_id means the row is its own root.
    pub async fn find_lineage_by_ids(
        conn: &mut PgConn,
        ids: &[SubscriptionAddOnId],
    ) -> DbResult<Vec<(SubscriptionAddOnId, Option<SubscriptionAddOnId>)>> {
        use crate::schema::subscription_add_on::dsl as d;

        if ids.is_empty() {
            return Ok(vec![]);
        }

        let query = d::subscription_add_on
            .filter(d::id.eq_any(ids))
            .select((d::id, d::lineage_id));

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while fetching subscription add-on lineage")
            .into_db_result()
    }

    pub async fn list_by_subscription_id(
        conn: &mut PgConn,
        tenant_id: &TenantId,
        subscription_id: &SubscriptionId,
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        let query = sao_dsl::subscription_add_on
            .inner_join(s_dsl::subscription)
            .filter(sao_dsl::subscription_id.eq(subscription_id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .select(SubscriptionAddOnRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing SubscriptionAddOn by subscription_id")
            .into_db_result()
    }

    /// Every add-on of the subscriptions, closed ones included.
    pub async fn list_all_by_subscriptions(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_ids: &[SubscriptionId],
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        if subscription_ids.is_empty() {
            return Ok(Vec::new());
        }
        let query = sao_dsl::subscription_add_on
            .inner_join(s_dsl::subscription)
            .filter(sao_dsl::subscription_id.eq_any(subscription_ids))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .select(SubscriptionAddOnRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing every add-on of subscriptions")
            .into_db_result()
    }

    /// List only currently-active add-ons (effective_to IS NULL).
    pub async fn list_by_subscription_id_active(
        conn: &mut PgConn,
        tenant_id: &TenantId,
        subscription_id: &SubscriptionId,
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        let query = sao_dsl::subscription_add_on
            .inner_join(s_dsl::subscription)
            .filter(sao_dsl::subscription_id.eq(subscription_id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(sao_dsl::effective_to.is_null())
            .select(SubscriptionAddOnRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing active SubscriptionAddOn by subscription_id")
            .into_db_result()
    }

    /// Close active add-ons by setting effective_to.
    pub async fn close_addons(
        conn: &mut PgConn,
        ids: &[SubscriptionAddOnId],
        effective_to: NaiveDate,
    ) -> DbResult<()> {
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        if ids.is_empty() {
            return Ok(());
        }

        let query = diesel::update(sao_dsl::subscription_add_on)
            .filter(sao_dsl::id.eq_any(ids))
            .filter(sao_dsl::effective_to.is_null())
            .set(sao_dsl::effective_to.eq(effective_to));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while closing subscription add-ons")
            .map(|_| ())
            .into_db_result()
    }

    /// List all add-ons (active and closed) overlapping with [period_start, period_end].
    pub async fn list_add_on_history_for_period(
        conn: &mut PgConn,
        subscription_id: &SubscriptionId,
        period_start: NaiveDate,
        period_end: NaiveDate,
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        // An add-on overlaps if: effective_from < period_end AND (effective_to IS NULL OR effective_to > period_start)
        let query = sao_dsl::subscription_add_on
            .filter(sao_dsl::subscription_id.eq(subscription_id))
            .filter(sao_dsl::effective_from.lt(period_end))
            .filter(
                sao_dsl::effective_to
                    .is_null()
                    .or(sao_dsl::effective_to.gt(period_start)),
            )
            .select(SubscriptionAddOnRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing add-on history for period")
            .into_db_result()
    }

    pub async fn list_active_add_on_ids(
        conn: &mut PgConn,
        subscription_ids: &[SubscriptionId],
        tenant_id: &TenantId,
    ) -> DbResult<Vec<common_domain::ids::AddOnId>> {
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_add_on::dsl as sao_dsl;
        use error_stack::ResultExt;

        if subscription_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = sao_dsl::subscription_add_on
            .inner_join(s_dsl::subscription)
            .filter(sao_dsl::subscription_id.eq_any(subscription_ids))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(sao_dsl::effective_to.is_null())
            .select(sao_dsl::add_on_id);

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while fetching add-on ids by subscription ids")
            .into_db_result()
    }

    /// Fetch distinct product IDs from active subscription add-ons for the given subscriptions.
    pub async fn list_active_product_ids(
        conn: &mut PgConn,
        subscription_ids: &[SubscriptionId],
        tenant_id: &TenantId,
    ) -> DbResult<Vec<ProductId>> {
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_add_on::dsl as sao_dsl;
        use diesel::dsl::not;
        use error_stack::ResultExt;

        if subscription_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = sao_dsl::subscription_add_on
            .inner_join(s_dsl::subscription)
            .filter(sao_dsl::subscription_id.eq_any(subscription_ids))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(sao_dsl::effective_to.is_null())
            .filter(not(sao_dsl::product_id.is_null()))
            .select(sao_dsl::product_id);

        crate::debug_sql(&query);

        let rows: Vec<Option<ProductId>> = query
            .get_results(conn)
            .await
            .attach("Error while fetching product ids from subscription add-ons")
            .into_db_result()?;

        Ok(rows.into_iter().flatten().collect())
    }
}

impl SubscriptionAddOnRow {
    /// The subscription of each of `ids` that belongs to the tenant.
    pub async fn subscription_ids_by_ids(
        conn: &mut PgConn,
        tenant_id: TenantId,
        ids: &[SubscriptionAddOnId],
    ) -> DbResult<std::collections::HashMap<SubscriptionAddOnId, SubscriptionId>> {
        use crate::schema::subscription::dsl as s;
        use crate::schema::subscription_add_on::dsl as d;
        use diesel::JoinOnDsl;

        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        d::subscription_add_on
            .inner_join(s::subscription.on(s::id.eq(d::subscription_id)))
            .filter(s::tenant_id.eq(tenant_id))
            .filter(d::id.eq_any(ids))
            .select((d::id, d::subscription_id))
            .load::<(SubscriptionAddOnId, SubscriptionId)>(conn)
            .await
            .attach("Error while reading the subscriptions of add-ons")
            .into_db_result()
            .map(|rows| rows.into_iter().collect())
    }
}
