use crate::customer_connection::{CustomerConnectionDetailsRow, CustomerConnectionRow};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::{CustomerConnectionId, CustomerId, TenantId};
use diesel::{ExpressionMethods, JoinOnDsl, QueryDsl, SelectableHelper, debug_query};
use error_stack::ResultExt;

impl CustomerConnectionRow {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<CustomerConnectionRow> {
        use crate::schema::customer_connection::dsl::customer_connection;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(customer_connection).values(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting customer to connector")
            .into_db_result()
    }

    pub async fn upsert(
        conn: &mut PgConn,
        tenant_id: &TenantId,
        row: CustomerConnectionRow,
    ) -> DbResult<CustomerConnectionRow> {
        use crate::schema::customer::dsl as cust_dsl;
        use crate::schema::customer_connection::dsl as cc_dsl;
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        // Verify the customer belongs to the tenant
        let customer_exists: bool = diesel::dsl::select(diesel::dsl::exists(
            cust_dsl::customer
                .filter(cust_dsl::id.eq(row.customer_id))
                .filter(cust_dsl::tenant_id.eq(tenant_id)),
        ))
        .get_result(conn)
        .await
        .attach("Error while checking customer ownership")
        .into_db_result()?;

        if !customer_exists {
            return Err(crate::errors::DatabaseErrorContainer {
                error: error_stack::Report::new(crate::errors::DatabaseError::ValidationError(
                    "Customer not found or unauthorized".into(),
                )),
            });
        }

        let query = diesel::insert_into(cc_dsl::customer_connection)
            .values(&row)
            .on_conflict((cc_dsl::customer_id, cc_dsl::connector_id))
            .do_update()
            .set(cc_dsl::external_customer_id.eq(&row.external_customer_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while upserting customer connection")
            .into_db_result()
    }

    pub async fn delete(
        conn: &mut PgConn,
        id: CustomerConnectionId,
        tenant_id: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::customer::dsl as cust_dsl;
        use crate::schema::customer_connection::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(c_dsl::customer_connection)
            .filter(c_dsl::id.eq(&id))
            .filter(diesel::dsl::exists(
                cust_dsl::customer
                    .filter(cust_dsl::id.eq(c_dsl::customer_id))
                    .filter(cust_dsl::tenant_id.eq(tenant_id))
                    .select(diesel::dsl::sql::<diesel::sql_types::Integer>("1")),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while deleting customer to connector")
            .into_db_result()
    }

    pub async fn get_by_id(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        id_param: &CustomerConnectionId,
    ) -> DbResult<CustomerConnectionRow> {
        use crate::schema::customer::dsl as cust_dsl;
        use crate::schema::customer_connection::dsl as ctc_dsl;
        use diesel_async::RunQueryDsl;

        let query = ctc_dsl::customer_connection
            .inner_join(cust_dsl::customer.on(ctc_dsl::customer_id.eq(cust_dsl::id)))
            .filter(cust_dsl::tenant_id.eq(tenant_id_param))
            .filter(ctc_dsl::id.eq(id_param))
            .select(CustomerConnectionRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while listing customer connectors by ids")
            .into_db_result()
    }
    pub async fn list_connections_by_customer_id(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        customer_id_param: &CustomerId,
    ) -> DbResult<Vec<CustomerConnectionRow>> {
        use crate::schema::customer::dsl as cust_dsl;
        use crate::schema::customer_connection::dsl as ctc_dsl;
        use diesel_async::RunQueryDsl;

        let query = ctc_dsl::customer_connection
            .inner_join(cust_dsl::customer.on(ctc_dsl::customer_id.eq(cust_dsl::id)))
            .filter(cust_dsl::tenant_id.eq(tenant_id_param))
            .filter(cust_dsl::id.eq(customer_id_param))
            .select(CustomerConnectionRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing customer connectors by ids")
            .into_db_result()
    }
    pub async fn list_connections_by_customer_ids(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        ids: Vec<CustomerId>,
    ) -> DbResult<Vec<CustomerConnectionRow>> {
        use crate::schema::customer::dsl as cust_dsl;
        use crate::schema::customer_connection::dsl as ctc_dsl;
        use diesel_async::RunQueryDsl;

        let query = ctc_dsl::customer_connection
            .inner_join(cust_dsl::customer.on(ctc_dsl::customer_id.eq(cust_dsl::id)))
            .filter(cust_dsl::tenant_id.eq(tenant_id_param))
            .filter(cust_dsl::id.eq_any(ids))
            .select(CustomerConnectionRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing customer connectors by ids")
            .into_db_result()
    }
}

impl CustomerConnectionDetailsRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        id_param: &CustomerConnectionId,
    ) -> DbResult<CustomerConnectionDetailsRow> {
        use crate::schema::connector::dsl as co_dsl;
        use crate::schema::customer::dsl as cu_dsl;
        use crate::schema::customer_connection::dsl as cc_dsl;
        use crate::schema::invoicing_entity::dsl as ie_dsl;
        use diesel_async::RunQueryDsl;

        let query = cc_dsl::customer_connection
            .inner_join(cu_dsl::customer.on(cc_dsl::customer_id.eq(cu_dsl::id)))
            .inner_join(co_dsl::connector.on(cc_dsl::connector_id.eq(co_dsl::id)))
            .inner_join(ie_dsl::invoicing_entity.on(cu_dsl::invoicing_entity_id.eq(ie_dsl::id)))
            .filter(cc_dsl::id.eq(id_param))
            .filter(cu_dsl::tenant_id.eq(tenant_id_param))
            .select(CustomerConnectionDetailsRow::as_select());
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while finding customer to connector by customer id")
            .into_db_result()
    }
}
