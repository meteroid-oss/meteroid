use crate::errors::IntoDbResult;
use std::collections::HashMap;

use crate::subscription_components::{SubscriptionComponentRow, SubscriptionComponentRowNew};
use crate::{DbResult, PgConn};

use diesel::debug_query;
use error_stack::ResultExt;

use common_domain::ids::{PriceComponentId, PriceId, SubscriptionId, SubscriptionPriceComponentId, TenantId};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use itertools::Itertools;

impl SubscriptionComponentRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SubscriptionComponentRow> {
        use crate::schema::subscription_component::dsl::subscription_component;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(subscription_component).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while inserting SubscriptionComponent batch")
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
            .select(SubscriptionComponentRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

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
            .select(SubscriptionComponentRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let res: Vec<SubscriptionComponentRow> = query
            .get_results(conn)
            .await
            .attach("Error while fetching SubscriptionComponents by subscriptions")
            .into_db_result()?;

        let grouped = res.into_iter().into_group_map_by(|c| c.subscription_id);
        Ok(grouped)
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
            .select(SubscriptionComponentRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

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
        use diesel::dsl::count;
        use diesel::AggregateExpressionMethods;
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
            .select(count(sc_dsl::subscription_id).aggregate_distinct());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while counting active subscriptions by product id")
            .into_db_result()
    }

    pub async fn list_by_price_ids(
        conn: &mut PgConn,
        price_ids: &[PriceId],
    ) -> DbResult<Vec<SubscriptionComponentRow>> {
        use crate::schema::subscription_component::dsl as sc_dsl;
        use diesel_async::RunQueryDsl;

        if price_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = sc_dsl::subscription_component
            .filter(sc_dsl::price_id.eq_any(price_ids))
            .select(SubscriptionComponentRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing subscription components by price ids")
            .into_db_result()
    }

    pub async fn update_price_id_and_fee(
        conn: &mut PgConn,
        component_id: SubscriptionPriceComponentId,
        new_price_id: PriceId,
        new_fee: serde_json::Value,
    ) -> DbResult<()> {
        use crate::schema::subscription_component::dsl as sc_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(sc_dsl::subscription_component)
            .filter(sc_dsl::id.eq(component_id))
            .set((
                sc_dsl::price_id.eq(Some(new_price_id)),
                sc_dsl::legacy_fee.eq(new_fee),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while updating subscription component price and fee")
            .map(|_| ())
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

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

        let query = diesel::delete(sc_dsl::subscription_component)
            .filter(sc_dsl::id.eq_any(ids));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while deleting subscription components")
            .map(|_| ())
            .into_db_result()
    }
}
