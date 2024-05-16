use diesel::{
    debug_query, ExpressionMethods, OptionalExtension, PgTextExpressionMethods, QueryDsl,
    SelectableHelper,
};
use error_stack::ResultExt;
use tap::TapFallible;

use crate::customers::{Customer, CustomerBrief, CustomerNew, CustomerPatch};
use crate::errors::IntoDbResult;
use crate::extend::order::OrderByRequest;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::{DbResult, PgConn};

impl CustomerNew {
    pub async fn insert(self, conn: &mut PgConn) -> DbResult<Customer> {
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

impl Customer {
    pub async fn find_by_id(conn: &mut PgConn, customer_id: uuid::Uuid) -> DbResult<Customer> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = customer.filter(id.eq(customer_id));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding customer by id")
            .into_db_result()
    }

    pub async fn find_by_alias(conn: &mut PgConn, customer_alias: String) -> DbResult<Customer> {
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
    ) -> DbResult<Vec<CustomerBrief>> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = customer
            .filter(tenant_id.eq(param_tenant_id))
            .filter(alias.eq_any(param_customer_aliases))
            .select(CustomerBrief::as_select());

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
    ) -> DbResult<PaginatedVec<Customer>> {
        use crate::schema::customer::dsl::*;

        let mut query = customer
            .filter(tenant_id.eq(param_tenant_id))
            .select(Customer::as_select())
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

    pub async fn insert_customer_batch(
        conn: &mut PgConn,
        batch: Vec<CustomerNew>,
    ) -> DbResult<Vec<Customer>> {
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
}

impl CustomerPatch {
    pub async fn update(&self, conn: &mut PgConn, _id: uuid::Uuid) -> DbResult<Option<Customer>> {
        use crate::schema::customer::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(customer).filter(id.eq(self.id)).set(self);

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
