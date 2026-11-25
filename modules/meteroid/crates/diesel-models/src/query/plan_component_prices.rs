use crate::errors::IntoDbResult;
use crate::plan_component_prices::{PlanComponentPriceRow, PlanComponentPriceRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{PlanOptionId, PriceComponentId};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use error_stack::ResultExt;

impl PlanComponentPriceRowNew {
    pub async fn insert_batch(
        conn: &mut PgConn,
        batch: &[PlanComponentPriceRowNew],
    ) -> DbResult<Vec<PlanComponentPriceRow>> {
        use crate::schema::plan_component_price::dsl::plan_component_price;
        use diesel_async::RunQueryDsl;

        if batch.is_empty() {
            return Ok(vec![]);
        }
        let query = diesel::insert_into(plan_component_price).values(batch);
        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while batch inserting plan component prices")
            .into_db_result()
    }
}

impl PlanComponentPriceRow {
    pub async fn list_by_component_ids(
        conn: &mut PgConn,
        component_ids: &[PriceComponentId],
    ) -> DbResult<Vec<PlanComponentPriceRow>> {
        use crate::schema::plan_component_price::dsl as pcp_dsl;
        use diesel_async::RunQueryDsl;

        let query = pcp_dsl::plan_component_price
            .filter(pcp_dsl::plan_component_id.eq_any(component_ids))
            .select(PlanComponentPriceRow::as_select());

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing plan component prices by component ids")
            .into_db_result()
    }

    /// Unlinks the component's prices but `keep`, in every option.
    pub async fn delete_by_component_except(
        conn: &mut PgConn,
        component_id: PriceComponentId,
        keep: &[common_domain::ids::PriceId],
    ) -> DbResult<()> {
        use crate::schema::plan_component_price::dsl as pcp_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(pcp_dsl::plan_component_price)
            .filter(pcp_dsl::plan_component_id.eq(component_id))
            .filter(diesel::dsl::not(pcp_dsl::price_id.eq_any(keep)));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while unlinking plan component prices")
            .map(|_| ())
            .into_db_result()
    }

    /// Unlinks `prices` from the component in every option; the price rows stay (another version
    /// may bill them).
    pub async fn delete_by_component_and_prices(
        conn: &mut PgConn,
        component_id: PriceComponentId,
        prices: &[common_domain::ids::PriceId],
    ) -> DbResult<()> {
        use crate::schema::plan_component_price::dsl as pcp_dsl;
        use diesel_async::RunQueryDsl;

        if prices.is_empty() {
            return Ok(());
        }
        let query = diesel::delete(pcp_dsl::plan_component_price)
            .filter(pcp_dsl::plan_component_id.eq(component_id))
            .filter(pcp_dsl::price_id.eq_any(prices));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while unlinking plan component prices")
            .map(|_| ())
            .into_db_result()
    }

    /// The links of `options`, for a new option copying another's cells.
    pub async fn list_by_option_ids(
        conn: &mut PgConn,
        option_ids: &[PlanOptionId],
    ) -> DbResult<Vec<PlanComponentPriceRow>> {
        use crate::schema::plan_component_price::dsl as pcp_dsl;
        use diesel_async::RunQueryDsl;

        if option_ids.is_empty() {
            return Ok(vec![]);
        }
        let query = pcp_dsl::plan_component_price
            .filter(pcp_dsl::option_id.eq_any(option_ids))
            .select(PlanComponentPriceRow::as_select());
        crate::debug_sql(&query);
        query
            .load(conn)
            .await
            .attach("Error while listing plan component prices by option ids")
            .into_db_result()
    }

    /// Unlinks `prices` from the component's cells of `option`; the price rows stay.
    pub async fn delete_by_component_option_and_prices(
        conn: &mut PgConn,
        component_id: PriceComponentId,
        option_id: PlanOptionId,
        prices: &[common_domain::ids::PriceId],
    ) -> DbResult<()> {
        use crate::schema::plan_component_price::dsl as pcp_dsl;
        use diesel_async::RunQueryDsl;

        if prices.is_empty() {
            return Ok(());
        }
        let query = diesel::delete(pcp_dsl::plan_component_price)
            .filter(pcp_dsl::plan_component_id.eq(component_id))
            .filter(pcp_dsl::option_id.eq(option_id))
            .filter(pcp_dsl::price_id.eq_any(prices));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while unlinking plan component prices")
            .map(|_| ())
            .into_db_result()
    }

    pub async fn delete_by_component_id(
        conn: &mut PgConn,
        component_id: PriceComponentId,
    ) -> DbResult<()> {
        use crate::schema::plan_component_price::dsl as pcp_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(pcp_dsl::plan_component_price)
            .filter(pcp_dsl::plan_component_id.eq(component_id));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting plan component prices by component id")
            .map(|_| ())
            .into_db_result()
    }
}
