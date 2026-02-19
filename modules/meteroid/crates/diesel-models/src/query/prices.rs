use crate::errors::IntoDbResult;
use crate::prices::{PriceRow, PriceRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{PriceId, ProductId, TenantId};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper, debug_query};
use error_stack::ResultExt;

impl PriceRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PriceRow> {
        use crate::schema::price::dsl::price;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(price).values(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting price")
            .into_db_result()
    }

    pub async fn insert_batch(conn: &mut PgConn, batch: &[PriceRowNew]) -> DbResult<Vec<PriceRow>> {
        use crate::schema::price::dsl::price;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(price).values(batch);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while batch inserting prices")
            .into_db_result()
    }
}

impl PriceRow {
    pub async fn find_by_id_and_tenant_id(
        conn: &mut PgConn,
        id: PriceId,
        tenant_id: TenantId,
    ) -> DbResult<PriceRow> {
        use crate::schema::price::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::price
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while finding price by id and tenant id")
            .into_db_result()
    }

    pub async fn list_by_product_id(
        conn: &mut PgConn,
        product_id: ProductId,
        tenant_id: TenantId,
    ) -> DbResult<Vec<PriceRow>> {
        use crate::schema::price::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::price
            .filter(p_dsl::product_id.eq(product_id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(p_dsl::archived_at.is_null())
            .select(PriceRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing prices by product id")
            .into_db_result()
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        ids: &[PriceId],
        tenant_id: TenantId,
    ) -> DbResult<Vec<PriceRow>> {
        use crate::schema::price::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::price
            .filter(p_dsl::id.eq_any(ids))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .select(PriceRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing prices by ids")
            .into_db_result()
    }

    pub async fn latest_by_product_ids_and_currency(
        conn: &mut PgConn,
        product_ids: &[ProductId],
        currency: &str,
        tenant_id: TenantId,
    ) -> DbResult<Vec<PriceRow>> {
        use crate::schema::price::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        if product_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = p_dsl::price
            .filter(p_dsl::product_id.eq_any(product_ids))
            .filter(p_dsl::currency.eq(currency))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(p_dsl::archived_at.is_null())
            .distinct_on(p_dsl::product_id)
            .order((p_dsl::product_id, p_dsl::created_at.desc()))
            .select(PriceRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing latest prices by product ids and currency")
            .into_db_result()
    }

    pub async fn update_pricing(
        conn: &mut PgConn,
        id: PriceId,
        tenant_id: TenantId,
        pricing: serde_json::Value,
    ) -> DbResult<PriceRow> {
        use crate::schema::price::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(p_dsl::price)
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .set(p_dsl::pricing.eq(pricing));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while updating price pricing")
            .into_db_result()
    }

    pub async fn archive(conn: &mut PgConn, id: PriceId, tenant_id: TenantId) -> DbResult<()> {
        use crate::schema::price::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let now = chrono::Utc::now().naive_utc();
        let query = diesel::update(p_dsl::price)
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .set(p_dsl::archived_at.eq(Some(now)));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while archiving price")
            .into_db_result()
            .map(|_| ())
    }
}
