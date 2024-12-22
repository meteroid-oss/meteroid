use crate::customers::{
    CustomerBriefRow, CustomerForDisplayRow, CustomerRow, CustomerRowNew, CustomerRowPatch,
};
use crate::errors::IntoDbResult;
use crate::extend::order::OrderByRequest;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::query::IdentityDb;
use crate::{DbResult, PgConn};
use diesel::{
    debug_query, BoolExpressionMethods, ExpressionMethods, JoinOnDsl, OptionalExtension,
    PgTextExpressionMethods, QueryDsl, SelectableHelper,
};
use error_stack::ResultExt;
use std::ops::Add;
use tap::TapFallible;
use uuid::Uuid;

impl CustomerRowNew {
    pub async fn insert(self, conn: &mut PgConn) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(customer).values(&self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting customer")
            .into_db_result()
    }
}

impl CustomerRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        customer_id: IdentityDb,
        tenant_id_param: Uuid,
    ) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let mut query = customer.filter(tenant_id.eq(tenant_id_param)).into_boxed();

        match customer_id {
            IdentityDb::UUID(id_param) => {
                query = query.filter(id.eq(id_param));
            }
            IdentityDb::LOCAL(local_id_param) => {
                query = query.filter(local_id.eq(local_id_param));
            }
        }

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding customer by id")
            .into_db_result()
    }

    pub async fn find_by_alias(conn: &mut PgConn, customer_alias: String) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = customer.filter(alias.eq(customer_alias));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding customer by alias")
            .into_db_result()
    }

    pub async fn find_by_aliases(
        conn: &mut PgConn,
        param_tenant_id: uuid::Uuid,
        param_customer_aliases: Vec<String>,
    ) -> DbResult<Vec<CustomerBriefRow>> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = customer
            .filter(tenant_id.eq(param_tenant_id))
            .filter(alias.eq_any(param_customer_aliases))
            .select(CustomerBriefRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while finding customer by aliases")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        param_tenant_id: uuid::Uuid,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
        param_query: Option<String>,
    ) -> DbResult<PaginatedVec<CustomerRow>> {
        use crate::schema::customer::dsl::*;

        let mut query = customer
            .filter(tenant_id.eq(param_tenant_id))
            .select(CustomerRow::as_select())
            .into_boxed();

        if let Some(param_query) = param_query {
            query = query.filter(
                name.ilike(format!("%{}%", param_query))
                    .or(alias.ilike(format!("%{}%", param_query))),
            );
        }

        match order_by {
            OrderByRequest::IdAsc => query = query.order(id.asc()),
            OrderByRequest::IdDesc => query = query.order(id.desc()),
            OrderByRequest::DateAsc => query = query.order(created_at.asc()),
            OrderByRequest::DateDesc => query = query.order(created_at.desc()),
            _ => query = query.order(id.asc()),
        }

        let paginated_query = query.paginate(pagination);

        log::debug!(
            "{}",
            debug_query::<diesel::pg::Pg, _>(&paginated_query).to_string()
        );

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while fetching customers")
            .into_db_result()
    }

    pub async fn list_by_ids(conn: &mut PgConn, ids: Vec<Uuid>) -> DbResult<Vec<CustomerRow>> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = customer
            .filter(id.eq_any(ids))
            .select(CustomerRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing customers by ids")
            .into_db_result()
    }

    pub async fn insert_customer_batch(
        conn: &mut PgConn,
        batch: Vec<CustomerRowNew>,
    ) -> DbResult<Vec<CustomerRow>> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(customer).values(&batch);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting customer batch")
            .into_db_result()
    }

    pub async fn select_for_update(
        conn: &mut PgConn,
        id: Uuid,
        tenant_id: Uuid,
    ) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::customer
            .for_no_key_update()
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while selecting for update customer by id")
            .into_db_result()
    }

    pub async fn update_balance(conn: &mut PgConn, id: Uuid, delta_cents: i32) -> DbResult<usize> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(c_dsl::customer)
            .filter(c_dsl::id.eq(id))
            .set((
                c_dsl::balance_value_cents.eq(c_dsl::balance_value_cents.add(delta_cents)),
                c_dsl::updated_at.eq(diesel::dsl::now),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while update customer balance")
            .into_db_result()
    }
}

impl CustomerRowPatch {
    pub async fn update(
        &self,
        conn: &mut PgConn,
        param_tenant_id: uuid::Uuid,
    ) -> DbResult<Option<CustomerRow>> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(customer)
            .filter(id.eq(self.id))
            .filter(tenant_id.eq(param_tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .optional()
            .tap_err(|e| log::error!("Error while updating customer: {:?}", e))
            .attach_printable("Error while updating customer")
            .into_db_result()
    }
}

impl CustomerForDisplayRow {
    pub async fn find_by_local_id_or_alias(
        conn: &mut PgConn,
        tenant_id: Uuid,
        local_id_or_alias: String,
    ) -> DbResult<CustomerForDisplayRow> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::invoicing_entity::dsl as ie_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::customer
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(
                c_dsl::local_id
                    .eq(local_id_or_alias.as_str())
                    .or(c_dsl::alias.eq(local_id_or_alias.as_str())),
            )
            .inner_join(ie_dsl::invoicing_entity.on(c_dsl::invoicing_entity_id.eq(ie_dsl::id)))
            .select(CustomerForDisplayRow::as_select());
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding customer by local_id or alias")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        param_tenant_id: Uuid,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
        param_query: Option<String>,
    ) -> DbResult<PaginatedVec<CustomerForDisplayRow>> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::invoicing_entity::dsl as ie_dsl;

        let mut query = c_dsl::customer
            .filter(c_dsl::tenant_id.eq(param_tenant_id))
            .inner_join(ie_dsl::invoicing_entity.on(c_dsl::invoicing_entity_id.eq(ie_dsl::id)))
            .select(CustomerForDisplayRow::as_select())
            .into_boxed();

        if let Some(param_query) = param_query {
            query = query.filter(
                c_dsl::name
                    .ilike(format!("%{}%", param_query))
                    .or(c_dsl::alias.ilike(format!("%{}%", param_query))),
            );
        }

        match order_by {
            OrderByRequest::IdAsc => query = query.order(c_dsl::id.asc()),
            OrderByRequest::IdDesc => query = query.order(c_dsl::id.desc()),
            OrderByRequest::DateAsc => query = query.order(c_dsl::created_at.asc()),
            OrderByRequest::DateDesc => query = query.order(c_dsl::created_at.desc()),
            _ => query = query.order(c_dsl::id.asc()),
        }

        let paginated_query = query.paginate(pagination);

        log::debug!(
            "{}",
            debug_query::<diesel::pg::Pg, _>(&paginated_query).to_string()
        );

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while fetching customers")
            .into_db_result()
    }
}
