use crate::errors::IntoDbResult;
use std::collections::HashMap;

use crate::subscription_components::{SubscriptionComponentRow, SubscriptionComponentRowNew};
use crate::{DbResult, PgConn};

use chrono::NaiveDate;
use error_stack::ResultExt;

use common_domain::ids::{
    PriceComponentId, PriceId, ProductId, SubscriptionId, SubscriptionPriceComponentId, TenantId,
};
use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl, SelectableHelper};
use itertools::Itertools;

impl SubscriptionComponentRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SubscriptionComponentRow> {
        use crate::schema::subscription_component::dsl::subscription_component;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(subscription_component).values(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting SubscriptionComponent")
            .into_db_result()
    }
}

impl SubscriptionComponentRow {
    pub async fn insert_subscription_component_batch(
        conn: &mut PgConn,
        batch: Vec<&SubscriptionComponentRowNew>,
    ) -> DbResult<Vec<SubscriptionComponentRow>> {
        use crate::schema::subscription_component::dsl::subscription_component;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(subscription_component).values(batch);

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while inserting SubscriptionComponent batch")
            .into_db_result()
    }

    /// Fetch the lineage root for a set of component ids. Returns `(id, lineage_id)`
    /// pairs; a `None` lineage_id means the row is its own root. Used to match an
    /// amendment credit to the originally-billed invoice line across overrides.
    pub async fn subscription_of(
        conn: &mut PgConn,
        id: SubscriptionPriceComponentId,
    ) -> DbResult<Option<SubscriptionId>> {
        use crate::schema::subscription_component::dsl as d;
        use diesel::OptionalExtension;
        use diesel_async::RunQueryDsl;

        d::subscription_component
            .filter(d::id.eq(id))
            .select(d::subscription_id)
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a subscription component's subscription")
            .into_db_result()
    }

    /// When the lineage `root` (the component and those continuing it) stopped: the latest end
    /// of its rows, or none while one is still active.
    pub async fn lineage_ended_on(
        conn: &mut PgConn,
        root: SubscriptionPriceComponentId,
    ) -> DbResult<Option<NaiveDate>> {
        use crate::schema::subscription_component::dsl as d;
        use diesel_async::RunQueryDsl;

        let ends: Vec<Option<NaiveDate>> = d::subscription_component
            .filter(d::id.eq(root).or(d::lineage_id.eq(root)))
            .select(d::effective_to)
            .load(conn)
            .await
            .attach("Error while reading a subscription component lineage's end")
            .into_db_result()?;
        Ok(if ends.iter().any(Option::is_none) {
            None
        } else {
            ends.into_iter().flatten().max()
        })
    }

    pub async fn find_lineage_by_ids(
        conn: &mut PgConn,
        ids: &[SubscriptionPriceComponentId],
    ) -> DbResult<
        Vec<(
            SubscriptionPriceComponentId,
            Option<SubscriptionPriceComponentId>,
        )>,
    > {
        use crate::schema::subscription_component::dsl as d;
        use diesel_async::RunQueryDsl;

        if ids.is_empty() {
            return Ok(vec![]);
        }

        let query = d::subscription_component
            .filter(d::id.eq_any(ids))
            .select((d::id, d::lineage_id));

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while fetching subscription component lineage")
            .into_db_result()
    }

    pub async fn list_subscription_components_by_subscription(
        conn: &mut PgConn,
        tenant_id_params: &TenantId,
        subscription_id: &SubscriptionId,
    ) -> DbResult<Vec<SubscriptionComponentRow>> {
        use crate::schema::subscription_component::dsl as subscription_component_dsl;
        use diesel_async::RunQueryDsl;

        let query = subscription_component_dsl::subscription_component
            .inner_join(crate::schema::subscription::table)
            .filter(subscription_component_dsl::subscription_id.eq(subscription_id))
            .filter(crate::schema::subscription::tenant_id.eq(tenant_id_params))
            .filter(subscription_component_dsl::effective_to.is_null())
            .select(SubscriptionComponentRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while fetching SubscriptionComponents by subscription")
            .into_db_result()
    }

    pub async fn list_subscription_components_by_subscriptions(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_ids: &[SubscriptionId],
    ) -> DbResult<HashMap<SubscriptionId, Vec<SubscriptionComponentRow>>> {
        use crate::schema::subscription_component::dsl as subscription_component_dsl;
        use diesel_async::RunQueryDsl;

        let query = subscription_component_dsl::subscription_component
            .filter(subscription_component_dsl::subscription_id.eq_any(subscription_ids))
            .inner_join(crate::schema::subscription::table)
            .filter(crate::schema::subscription::tenant_id.eq(tenant_id_param))
            .filter(subscription_component_dsl::effective_to.is_null())
            .select(SubscriptionComponentRow::as_select());

        crate::debug_sql(&query);

        let res: Vec<SubscriptionComponentRow> = query
            .get_results(conn)
            .await
            .attach("Error while fetching SubscriptionComponents by subscriptions")
            .into_db_result()?;

        let grouped = res.into_iter().into_group_map_by(|c| c.subscription_id);
        Ok(grouped)
    }

    /// Every component of the subscriptions, closed ones included.
    pub async fn list_all_by_subscriptions(
        conn: &mut PgConn,
        tenant_id_param: TenantId,
        subscription_ids: &[SubscriptionId],
    ) -> DbResult<Vec<SubscriptionComponentRow>> {
        use crate::schema::subscription_component::dsl as subscription_component_dsl;
        use diesel_async::RunQueryDsl;

        if subscription_ids.is_empty() {
            return Ok(Vec::new());
        }
        let query = subscription_component_dsl::subscription_component
            .filter(subscription_component_dsl::subscription_id.eq_any(subscription_ids))
            .inner_join(crate::schema::subscription::table)
            .filter(crate::schema::subscription::tenant_id.eq(tenant_id_param))
            .select(SubscriptionComponentRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing every component of subscriptions")
            .into_db_result()
    }

    pub async fn list_by_product_id(
        conn: &mut PgConn,
        product_id: &common_domain::ids::ProductId,
        tenant_id: &TenantId,
    ) -> DbResult<Vec<SubscriptionComponentRow>> {
        use crate::schema::subscription_component::dsl as sc_dsl;
        use diesel_async::RunQueryDsl;

        let query = sc_dsl::subscription_component
            .inner_join(crate::schema::subscription::table)
            .filter(sc_dsl::product_id.eq(product_id))
            .filter(crate::schema::subscription::tenant_id.eq(tenant_id))
            .filter(sc_dsl::effective_to.is_null())
            .select(SubscriptionComponentRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing subscription components by product id")
            .into_db_result()
    }

    pub async fn count_active_subscriptions_by_product_id(
        conn: &mut PgConn,
        product_id: &common_domain::ids::ProductId,
        tenant_id: &TenantId,
    ) -> DbResult<i64> {
        use crate::enums::SubscriptionStatusEnum;
        use crate::schema::subscription_component::dsl as sc_dsl;
        use diesel::AggregateExpressionMethods;
        use diesel::dsl::count;
        use diesel_async::RunQueryDsl;

        let active_statuses = vec![
            SubscriptionStatusEnum::PendingActivation,
            SubscriptionStatusEnum::PendingCharge,
            SubscriptionStatusEnum::TrialActive,
            SubscriptionStatusEnum::Active,
        ];

        let query = sc_dsl::subscription_component
            .inner_join(crate::schema::subscription::table)
            .filter(sc_dsl::product_id.eq(product_id))
            .filter(crate::schema::subscription::tenant_id.eq(tenant_id))
            .filter(crate::schema::subscription::status.eq_any(active_statuses))
            .filter(sc_dsl::effective_to.is_null())
            .select(count(sc_dsl::subscription_id).aggregate_distinct());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while counting active subscriptions by product id")
            .into_db_result()
    }

    pub async fn update_for_plan_change(
        conn: &mut PgConn,
        component_id: SubscriptionPriceComponentId,
        new_price_component_id: PriceComponentId,
        new_price_id: Option<PriceId>,
        new_name: String,
        new_fee: serde_json::Value,
        new_period: crate::enums::SubscriptionFeeBillingPeriod,
    ) -> DbResult<()> {
        use crate::schema::subscription_component::dsl as sc_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(sc_dsl::subscription_component)
            .filter(sc_dsl::id.eq(component_id))
            .set((
                sc_dsl::price_component_id.eq(Some(new_price_component_id)),
                sc_dsl::price_id.eq(new_price_id),
                sc_dsl::name.eq(new_name),
                sc_dsl::legacy_fee.eq(new_fee),
                sc_dsl::period.eq(new_period),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while updating subscription component for plan change")
            .map(|_| ())
            .into_db_result()
    }

    pub async fn delete_by_ids(
        conn: &mut PgConn,
        ids: &[SubscriptionPriceComponentId],
    ) -> DbResult<()> {
        use crate::schema::subscription_component::dsl as sc_dsl;
        use diesel_async::RunQueryDsl;

        if ids.is_empty() {
            return Ok(());
        }

        let query = diesel::delete(sc_dsl::subscription_component).filter(sc_dsl::id.eq_any(ids));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting subscription components")
            .map(|_| ())
            .into_db_result()
    }

    /// Close active components by setting effective_to.
    pub async fn close_components(
        conn: &mut PgConn,
        ids: &[SubscriptionPriceComponentId],
        effective_to: NaiveDate,
    ) -> DbResult<()> {
        use crate::schema::subscription_component::dsl as sc_dsl;
        use diesel_async::RunQueryDsl;

        if ids.is_empty() {
            return Ok(());
        }

        let query = diesel::update(sc_dsl::subscription_component)
            .filter(sc_dsl::id.eq_any(ids))
            .filter(sc_dsl::effective_to.is_null())
            .set(sc_dsl::effective_to.eq(effective_to));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while closing subscription components")
            .map(|_| ())
            .into_db_result()
    }

    /// List all components (active and closed) overlapping with [period_start, period_end].
    pub async fn list_component_history_for_period(
        conn: &mut PgConn,
        subscription_id: &SubscriptionId,
        period_start: NaiveDate,
        period_end: NaiveDate,
    ) -> DbResult<Vec<SubscriptionComponentRow>> {
        use crate::schema::subscription_component::dsl as sc_dsl;
        use diesel_async::RunQueryDsl;

        // A component overlaps if: effective_from < period_end AND (effective_to IS NULL OR effective_to > period_start)
        let query = sc_dsl::subscription_component
            .filter(sc_dsl::subscription_id.eq(subscription_id))
            .filter(sc_dsl::effective_from.lt(period_end))
            .filter(
                sc_dsl::effective_to
                    .is_null()
                    .or(sc_dsl::effective_to.gt(period_start)),
            )
            .select(SubscriptionComponentRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing component history for period")
            .into_db_result()
    }

    /// Fetch distinct product IDs from subscription components for the given subscriptions.
    pub async fn list_product_ids(
        conn: &mut PgConn,
        subscription_ids: &[SubscriptionId],
        tenant_id: &TenantId,
    ) -> DbResult<Vec<ProductId>> {
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_component::dsl as sc_dsl;
        use diesel::dsl::not;
        use diesel_async::RunQueryDsl;

        if subscription_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = sc_dsl::subscription_component
            .inner_join(s_dsl::subscription)
            .filter(sc_dsl::subscription_id.eq_any(subscription_ids))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(sc_dsl::effective_to.is_null())
            .filter(not(sc_dsl::product_id.is_null()))
            .select(sc_dsl::product_id);

        crate::debug_sql(&query);

        let rows: Vec<Option<ProductId>> = query
            .get_results(conn)
            .await
            .attach("Error while fetching product ids from subscription components")
            .into_db_result()?;

        Ok(rows.into_iter().flatten().collect())
    }
}

impl SubscriptionComponentRow {
    /// The subscription of each of `ids` that belongs to the tenant, historical rows included.
    pub async fn subscription_ids_by_ids(
        conn: &mut PgConn,
        tenant_id: TenantId,
        ids: &[SubscriptionPriceComponentId],
    ) -> DbResult<HashMap<SubscriptionPriceComponentId, SubscriptionId>> {
        use crate::schema::subscription::dsl as s;
        use crate::schema::subscription_component::dsl as d;
        use diesel::JoinOnDsl;
        use diesel_async::RunQueryDsl;

        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        d::subscription_component
            .inner_join(s::subscription.on(s::id.eq(d::subscription_id)))
            .filter(s::tenant_id.eq(tenant_id))
            .filter(d::id.eq_any(ids))
            .select((d::id, d::subscription_id))
            .load::<(SubscriptionPriceComponentId, SubscriptionId)>(conn)
            .await
            .attach("Error while reading the subscriptions of components")
            .into_db_result()
            .map(|rows| rows.into_iter().collect())
    }

    /// Of these components and add-ons, the ones whose fee is a one-time (`once`) credit pack.
    pub async fn once_credit_of(
        conn: &mut PgConn,
        tenant_id: TenantId,
        components: &[SubscriptionPriceComponentId],
        add_ons: &[common_domain::ids::SubscriptionAddOnId],
    ) -> DbResult<std::collections::HashSet<uuid::Uuid>> {
        use diesel::sql_types;
        use diesel_async::RunQueryDsl;

        #[derive(diesel::QueryableByName)]
        struct Once {
            #[diesel(sql_type = sql_types::Uuid)]
            id: uuid::Uuid,
        }

        if components.is_empty() && add_ons.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        let components: Vec<uuid::Uuid> = components.iter().map(|id| **id).collect();
        let add_ons: Vec<uuid::Uuid> = add_ons.iter().map(|id| **id).collect();
        diesel::sql_query(
            r#"
            SELECT sc.id
            FROM subscription_component sc
            JOIN subscription s ON s.id = sc.subscription_id
            WHERE s.tenant_id = $1 AND sc.id = ANY($2)
              AND sc.legacy_fee @> '{"Credit": {"once": true}}'
            UNION ALL
            SELECT sa.id
            FROM subscription_add_on sa
            JOIN subscription s ON s.id = sa.subscription_id
            WHERE s.tenant_id = $1 AND sa.id = ANY($3)
              AND sa.legacy_fee @> '{"Credit": {"once": true}}'
            "#,
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Array<sql_types::Uuid>, _>(components)
        .bind::<sql_types::Array<sql_types::Uuid>, _>(add_ons)
        .load::<Once>(conn)
        .await
        .attach("Error while reading the one-time credit packs of components")
        .into_db_result()
        .map(|rows| rows.into_iter().map(|row| row.id).collect())
    }
}
