use crate::errors::IntoDbResult;
use crate::plan_component_prices::{PlanComponentPriceRow, PlanComponentPriceRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{PriceComponentId, PriceId};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper, debug_query};
use error_stack::ResultExt;

impl PlanComponentPriceRowNew {
    pub async fn insert_batch(
        conn: &mut PgConn,
        batch: &[PlanComponentPriceRowNew],
    ) -> DbResult<Vec<PlanComponentPriceRow>> {
        use crate::schema::plan_component_price::dsl::plan_component_price;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(plan_component_price).values(batch);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing plan component prices by component ids")
            .into_db_result()
    }

    pub async fn list_by_price_ids(
        conn: &mut PgConn,
        price_ids: &[PriceId],
    ) -> DbResult<Vec<PlanComponentPriceRow>> {
        use crate::schema::plan_component_price::dsl as pcp_dsl;
        use diesel_async::RunQueryDsl;

        if price_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = pcp_dsl::plan_component_price
            .filter(pcp_dsl::price_id.eq_any(price_ids))
            .select(PlanComponentPriceRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing plan component prices by price ids")
            .into_db_result()
    }

    pub async fn delete_by_price_ids(
        conn: &mut PgConn,
        price_ids: &[PriceId],
    ) -> DbResult<()> {
        use crate::schema::plan_component_price::dsl as pcp_dsl;
        use diesel_async::RunQueryDsl;

        if price_ids.is_empty() {
            return Ok(());
        }

        let query = diesel::delete(pcp_dsl::plan_component_price)
            .filter(pcp_dsl::price_id.eq_any(price_ids));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while deleting plan component prices by price ids")
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while deleting plan component prices by component id")
            .map(|_| ())
            .into_db_result()
    }
}
