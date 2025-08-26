use crate::customers::{
    CustomerBriefRow, CustomerRow, CustomerRowNew, CustomerRowPatch, CustomerRowUpdate,
};
use crate::enums::ConnectorProviderEnum;
use crate::errors::IntoDbResult;
use crate::extend::connection_metadata;
use crate::extend::order::OrderByRequest;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::{DbResult, PgConn};
use common_domain::ids::{AliasOr, BaseId, ConnectorId, CustomerId, TenantId};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, OptionalExtension, PgTextExpressionMethods, QueryDsl,
    SelectableHelper, debug_query,
};
use error_stack::ResultExt;
use itertools::Itertools;
use std::ops::Add;
use tap::TapFallible;
use uuid::Uuid;

impl CustomerRowNew {
    pub async fn insert(self, conn: &mut PgConn) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(customer).values(&self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting customer")
            .into_db_result()
    }
}

impl CustomerRow {
    pub async fn find_by_ids_or_aliases(
        conn: &mut PgConn,
        tenant_id: TenantId,
        ids_or_aliases: Vec<AliasOr<CustomerId>>,
    ) -> DbResult<Vec<CustomerRow>> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let (ids, aliases): (Vec<CustomerId>, Vec<String>) = ids_or_aliases
            .into_iter()
            .partition_map(|id_or_alias| match id_or_alias {
                AliasOr::Id(id) => itertools::Either::Left(id),
                AliasOr::Alias(alias) => itertools::Either::Right(alias),
            });

        let query = c_dsl::customer
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(c_dsl::archived_at.is_null())
            .filter(c_dsl::id.eq_any(ids).or(c_dsl::alias.eq_any(aliases)))
            .select(CustomerRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while finding customers by ids or aliases")
            .into_db_result()
    }

    pub async fn find_by_id_or_alias(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id_or_alias: AliasOr<CustomerId>,
    ) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = c_dsl::customer
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(c_dsl::archived_at.is_null())
            .select(CustomerRow::as_select())
            .into_boxed();

        match id_or_alias {
            AliasOr::Id(id) => {
                query = query.filter(c_dsl::id.eq(id));
            }
            AliasOr::Alias(alias) => {
                query = query.filter(c_dsl::alias.eq(alias));
            }
        }

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding customer by id or alias")
            .into_db_result()
    }

    pub async fn find_by_id(
        conn: &mut PgConn,
        customer_id: &CustomerId,
        tenant_id_param: &TenantId,
    ) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::customer
            .filter(c_dsl::id.eq(customer_id))
            .filter(c_dsl::tenant_id.eq(tenant_id_param))
            .filter(c_dsl::archived_at.is_null());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding customer by id")
            .into_db_result()
    }

    pub async fn find_by_alias(conn: &mut PgConn, customer_alias: String) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = customer
            .filter(alias.eq(customer_alias))
            .filter(archived_at.is_null());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding customer by alias")
            .into_db_result()
    }

    pub async fn find_by_aliases(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_customer_aliases: Vec<String>,
    ) -> DbResult<Vec<CustomerBriefRow>> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = customer
            .filter(tenant_id.eq(param_tenant_id))
            .filter(alias.eq_any(param_customer_aliases))
            .filter(archived_at.is_null())
            .select(CustomerBriefRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while finding customer by aliases")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
        param_query: Option<String>,
    ) -> DbResult<PaginatedVec<CustomerRow>> {
        use crate::schema::customer::dsl::*;

        let mut query = customer
            .filter(tenant_id.eq(param_tenant_id))
            .filter(archived_at.is_null())
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&paginated_query));

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while fetching customers")
            .into_db_result()
    }

    pub async fn list_by_ids_global(
        conn: &mut PgConn,
        ids: Vec<CustomerId>,
    ) -> DbResult<Vec<CustomerRow>> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = customer
            .filter(id.eq_any(ids))
            .filter(archived_at.is_null())
            .select(CustomerRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing customers by ids")
            .into_db_result()
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        ids: Vec<CustomerId>,
    ) -> DbResult<Vec<CustomerRow>> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = customer
            .filter(tenant_id.eq(tenant_id_param))
            .filter(id.eq_any(ids))
            .select(CustomerRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

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
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting customer batch")
            .into_db_result()
    }

    pub async fn select_for_update(
        conn: &mut PgConn,
        id: CustomerId,
        tenant_id: TenantId,
    ) -> DbResult<CustomerRow> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::customer
            .for_no_key_update()
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while selecting for update customer by id")
            .into_db_result()
    }

    pub async fn update_balance(
        conn: &mut PgConn,
        id: CustomerId,
        delta_cents: i64,
    ) -> DbResult<usize> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(c_dsl::customer)
            .filter(c_dsl::id.eq(id))
            .set((
                c_dsl::balance_value_cents.eq(c_dsl::balance_value_cents.add(delta_cents)),
                c_dsl::updated_at.eq(diesel::dsl::now),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while update customer balance")
            .into_db_result()
    }

    pub async fn archive(
        conn: &mut PgConn,
        id: CustomerId,
        tenant_id: TenantId,
        archived_by: Uuid,
    ) -> DbResult<usize> {
        use crate::schema::customer::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(c_dsl::customer)
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .set((
                c_dsl::archived_at.eq(diesel::dsl::now),
                c_dsl::archived_by.eq(archived_by),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while archiving customer")
            .into_db_result()
    }
}

impl CustomerRowPatch {
    pub async fn update(
        &self,
        conn: &mut PgConn,
        param_tenant_id: TenantId,
    ) -> DbResult<Option<CustomerRow>> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(customer)
            .filter(id.eq(self.id))
            .filter(tenant_id.eq(param_tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .optional()
            .tap_err(|e| log::error!("Error while patching customer: {:?}", e))
            .attach_printable("Error while patching customer")
            .into_db_result()
    }

    pub async fn upsert_conn_meta(
        conn: &mut PgConn,
        provider: ConnectorProviderEnum,
        customer_id: CustomerId,
        connector_id: ConnectorId,
        external_id: &str,
        external_company_id: &str,
    ) -> DbResult<()> {
        connection_metadata::upsert(
            conn,
            "customer",
            provider.as_meta_key(),
            customer_id.as_uuid(),
            connector_id,
            external_id,
            external_company_id,
        )
        .await
        .map(|_| ())
    }
}

impl CustomerRowUpdate {
    pub async fn update(
        &self,
        conn: &mut PgConn,
        param_tenant_id: TenantId,
    ) -> DbResult<Option<CustomerRow>> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(customer)
            .filter(id.eq(self.id))
            .filter(tenant_id.eq(param_tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .optional()
            .tap_err(|e| log::error!("Error while updating customer: {:?}", e))
            .attach_printable("Error while updating customer")
            .into_db_result()
    }
}
