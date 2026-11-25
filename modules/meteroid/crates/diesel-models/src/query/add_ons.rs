use crate::add_ons::{AddOnPriceRow, AddOnRow, AddOnRowNew, AddOnRowPatch};
use crate::errors::IntoDbResult;
use crate::extend::order::{OrderByParam, OrderDirection};
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::{DbResult, PgConn};
use common_domain::ids::{AddOnId, PlanVersionId, PriceId, ProductId, TenantId};
use diesel::{ExpressionMethods, PgTextExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use std::collections::HashMap;
use tap::TapFallible;

impl AddOnRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<AddOnRow> {
        use crate::schema::add_on::dsl as ao_dsl;

        let query = diesel::insert_into(ao_dsl::add_on).values(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting add-on")
            .into_db_result()
    }
}

impl AddOnRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: AddOnId,
    ) -> DbResult<AddOnRow> {
        use crate::schema::add_on::dsl as ao_dsl;

        let query = ao_dsl::add_on
            .filter(ao_dsl::id.eq(id))
            .filter(ao_dsl::tenant_id.eq(tenant_id));

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while getting add-on")
            .into_db_result()
    }

    /// List add-ons for a tenant. If `plan_version_id` is provided, filters through the junction table.
    /// If `currency` is provided, filters add-ons priced in that currency.
    pub async fn list_by_tenant_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        plan_version_id: Option<PlanVersionId>,
        pagination: PaginationRequest,
        search: Option<String>,
        currency: Option<String>,
        include_archived: bool,
        order_by: Option<&str>,
    ) -> DbResult<PaginatedVec<AddOnRow>> {
        use crate::schema::add_on::dsl as ao_dsl;

        let mut query = ao_dsl::add_on
            .filter(ao_dsl::tenant_id.eq(tenant_id))
            .into_boxed();

        if !include_archived {
            query = query.filter(ao_dsl::archived_at.is_null());
        }

        if let Some(pv_id) = plan_version_id {
            use crate::schema::plan_version_add_on::dsl as pva_dsl;

            let add_on_ids_subquery = pva_dsl::plan_version_add_on
                .filter(pva_dsl::plan_version_id.eq(pv_id))
                .select(pva_dsl::add_on_id);

            query = query.filter(ao_dsl::id.eq_any(add_on_ids_subquery));
        }

        if let Some(currency) = currency {
            use crate::schema::add_on_price::dsl as aop_dsl;
            use crate::schema::price::dsl as p_dsl;

            let priced_subquery = aop_dsl::add_on_price
                .inner_join(p_dsl::price)
                .filter(p_dsl::currency.eq(currency))
                .select(aop_dsl::add_on_id);

            query = query.filter(ao_dsl::id.eq_any(priced_subquery));
        }

        if let Some(search) = search {
            query = query.filter(ao_dsl::name.ilike(format!("%{search}%")));
        }

        let mut query = query.select(AddOnRow::as_select());

        let order = OrderByParam::parse(order_by, "created_at.desc");

        match (order.column.as_str(), order.direction) {
            ("name", OrderDirection::Asc) => {
                query = query.order((ao_dsl::name.asc(), ao_dsl::id.asc()))
            }
            ("name", OrderDirection::Desc) => {
                query = query.order((ao_dsl::name.desc(), ao_dsl::id.desc()))
            }
            ("created_at", OrderDirection::Asc) => {
                query = query.order((ao_dsl::created_at.asc(), ao_dsl::id.asc()))
            }
            ("created_at", OrderDirection::Desc) => {
                query = query.order((ao_dsl::created_at.desc(), ao_dsl::id.desc()))
            }
            _ => query = query.order((ao_dsl::created_at.desc(), ao_dsl::id.desc())),
        }

        let query = query.paginate(pagination);

        crate::debug_sql(&query);

        query
            .load_and_count_pages(conn)
            .await
            .tap_err(|e| log::error!("Error while listing add-ons: {e:?}"))
            .attach("Error while listing add-ons")
            .into_db_result()
    }

    pub async fn archive(conn: &mut PgConn, id: AddOnId, tenant_id: TenantId) -> DbResult<()> {
        use crate::schema::add_on::dsl as ao_dsl;

        let query = diesel::update(ao_dsl::add_on)
            .filter(ao_dsl::id.eq(id))
            .filter(ao_dsl::tenant_id.eq(tenant_id))
            .filter(ao_dsl::archived_at.is_null())
            .set(ao_dsl::archived_at.eq(common_clock::now().naive_utc()));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error while archiving add-on: {e:?}"))
            .attach("Error while archiving add-on")
            .into_db_result()?;

        Ok(())
    }

    pub async fn unarchive(conn: &mut PgConn, id: AddOnId, tenant_id: TenantId) -> DbResult<()> {
        use crate::schema::add_on::dsl as ao_dsl;

        let query = diesel::update(ao_dsl::add_on)
            .filter(ao_dsl::id.eq(id))
            .filter(ao_dsl::tenant_id.eq(tenant_id))
            .set(ao_dsl::archived_at.eq::<Option<chrono::NaiveDateTime>>(None));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error while unarchiving add-on: {e:?}"))
            .attach("Error while unarchiving add-on")
            .into_db_result()?;

        Ok(())
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        ids: &[AddOnId],
        tenant_id: &TenantId,
    ) -> DbResult<Vec<AddOnRow>> {
        use crate::schema::add_on::dsl as ao_dsl;

        let query = ao_dsl::add_on
            .filter(ao_dsl::id.eq_any(ids))
            .filter(ao_dsl::tenant_id.eq(tenant_id))
            .filter(ao_dsl::archived_at.is_null());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while fetching add-ons: {e:?}"))
            .attach("Error while fetching add-ons")
            .into_db_result()
    }

    /// The live add-ons selling `product_id`, by name.
    pub async fn list_live_by_product(
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_id: ProductId,
    ) -> DbResult<Vec<AddOnRow>> {
        use crate::schema::add_on::dsl as ao_dsl;

        let query = ao_dsl::add_on
            .filter(ao_dsl::product_id.eq(product_id))
            .filter(ao_dsl::tenant_id.eq(tenant_id))
            .filter(ao_dsl::archived_at.is_null())
            .order((ao_dsl::name.asc(), ao_dsl::id.asc()))
            .select(AddOnRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing the add-ons of a product")
            .into_db_result()
    }
}

impl AddOnRowPatch {
    pub async fn patch(&self, conn: &mut PgConn) -> DbResult<AddOnRow> {
        use crate::schema::add_on::dsl as ao_dsl;

        let query = diesel::update(ao_dsl::add_on)
            .filter(ao_dsl::id.eq(self.id))
            .filter(ao_dsl::tenant_id.eq(self.tenant_id))
            .set(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while updating add-on")
            .into_db_result()
    }
}

impl AddOnPriceRow {
    pub async fn insert_batch(conn: &mut PgConn, rows: &[AddOnPriceRow]) -> DbResult<usize> {
        use crate::schema::add_on_price::dsl as aop_dsl;

        if rows.is_empty() {
            return Ok(0);
        }

        let query = diesel::insert_into(aop_dsl::add_on_price)
            .values(rows)
            .on_conflict_do_nothing();

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while inserting add-on prices")
            .into_db_result()
    }

    /// The price ids of each add-on of `add_on_ids`.
    pub async fn list_by_add_on_ids(
        conn: &mut PgConn,
        add_on_ids: &[AddOnId],
    ) -> DbResult<HashMap<AddOnId, Vec<PriceId>>> {
        use crate::schema::add_on_price::dsl as aop_dsl;

        if add_on_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let query = aop_dsl::add_on_price
            .filter(aop_dsl::add_on_id.eq_any(add_on_ids))
            .select(AddOnPriceRow::as_select());

        crate::debug_sql(&query);

        let rows: Vec<AddOnPriceRow> = query
            .load(conn)
            .await
            .attach("Error while listing add-on prices")
            .into_db_result()?;

        let mut by_add_on: HashMap<AddOnId, Vec<PriceId>> = HashMap::new();
        for row in rows {
            by_add_on
                .entry(row.add_on_id)
                .or_default()
                .push(row.price_id);
        }
        Ok(by_add_on)
    }

    /// Unlinks the prices of `add_on_id` not in `kept`; the price rows stay, as subscriptions
    /// bought at them still read them.
    pub async fn delete_except(
        conn: &mut PgConn,
        add_on_id: AddOnId,
        kept: &[PriceId],
    ) -> DbResult<usize> {
        use crate::schema::add_on_price::dsl as aop_dsl;

        let query = diesel::delete(
            aop_dsl::add_on_price
                .filter(aop_dsl::add_on_id.eq(add_on_id))
                .filter(diesel::dsl::not(aop_dsl::price_id.eq_any(kept))),
        );

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while unlinking add-on prices")
            .into_db_result()
    }
}
