use crate::customer_payment_methods::{
    CustomerPaymentMethodRow, CustomerPaymentMethodRowNew, CustomerPaymentMethodWithConnector,
    ResolvedSubscriptionPaymentContext,
};

use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::{
    ConnectorId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId, SubscriptionId,
    TenantId,
};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper, debug_query};
use error_stack::ResultExt;

impl CustomerPaymentMethodRow {
    pub async fn delete(
        conn: &mut PgConn,
        id_param: &CustomerPaymentMethodId,
        tenant_id_param: &TenantId,
    ) -> DbResult<usize> {
        use crate::schema::customer_payment_method::dsl::{customer_payment_method, id, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(customer_payment_method)
            .filter(id.eq(id_param))
            .filter(tenant_id.eq(tenant_id_param));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while deleting customer payment method")
            .into_db_result()
    }

    pub async fn get_by_id(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        payment_method_id_param: &CustomerPaymentMethodId,
    ) -> DbResult<CustomerPaymentMethodRow> {
        use crate::schema::customer_payment_method::dsl as cpm_dsl;
        use diesel_async::RunQueryDsl;

        let query = cpm_dsl::customer_payment_method
            .filter(cpm_dsl::id.eq(payment_method_id_param))
            .filter(cpm_dsl::tenant_id.eq(tenant_id_param))
            .select(CustomerPaymentMethodRow::as_select());
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while finding customer payment method by id")
            .into_db_result()
    }

    pub async fn get_subscription_payment_context(
        conn: &mut PgConn,
        tenant_id_param: TenantId,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<ResolvedSubscriptionPaymentContext> {
        use crate::schema::customer::dsl as cust_dsl;
        use crate::schema::invoicing_entity::dsl as ie_dsl;
        use crate::schema::subscription::dsl as sub_dsl;
        use diesel_async::RunQueryDsl;

        let query = sub_dsl::subscription
            .inner_join(cust_dsl::customer.inner_join(ie_dsl::invoicing_entity))
            .filter(sub_dsl::id.eq(subscription_id_param))
            .filter(sub_dsl::tenant_id.eq(tenant_id_param))
            .select(ResolvedSubscriptionPaymentContext::as_select());
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while fetching payment context for subscription")
            .into_db_result()
    }

    pub async fn list_customer_payment_methods_by_providers(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        customer_id_param: &CustomerId,
        provider_ids: &[ConnectorId],
    ) -> DbResult<Vec<CustomerPaymentMethodWithConnector>> {
        use crate::schema::customer_connection::dsl as cc_dsl;
        use crate::schema::customer_payment_method::dsl as cpm_dsl;
        use diesel_async::RunQueryDsl;

        if provider_ids.is_empty() {
            return Ok(vec![]);
        }

        let query = cpm_dsl::customer_payment_method
            .inner_join(cc_dsl::customer_connection)
            .filter(cpm_dsl::customer_id.eq(customer_id_param))
            .filter(cpm_dsl::tenant_id.eq(tenant_id_param))
            .filter(cpm_dsl::archived_at.is_null())
            .filter(cc_dsl::connector_id.eq_any(provider_ids))
            .select(CustomerPaymentMethodWithConnector::as_select());
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing customer payment methods by providers")
            .into_db_result()
    }

    pub async fn list_by_connection_id(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        connection_id_param: &CustomerConnectionId,
    ) -> DbResult<Vec<CustomerPaymentMethodRow>> {
        use crate::schema::customer_payment_method::dsl as cpm_dsl;
        use diesel_async::RunQueryDsl;

        let query = cpm_dsl::customer_payment_method
            .filter(cpm_dsl::connection_id.eq(connection_id_param))
            .filter(cpm_dsl::tenant_id.eq(tenant_id_param))
            .select(CustomerPaymentMethodRow::as_select());
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while finding customer payment methods by connection id")
            .into_db_result()
    }

    pub async fn list_by_customer_id(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        customer_id_param: &CustomerId,
    ) -> DbResult<Vec<CustomerPaymentMethodRow>> {
        use crate::schema::customer_payment_method::dsl as cpm_dsl;
        use diesel_async::RunQueryDsl;

        let query = cpm_dsl::customer_payment_method
            .filter(cpm_dsl::customer_id.eq(customer_id_param))
            .filter(cpm_dsl::tenant_id.eq(tenant_id_param))
            .select(CustomerPaymentMethodRow::as_select());
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while finding customer payment methods by connection id")
            .into_db_result()
    }
}

impl CustomerPaymentMethodRowNew {
    pub async fn upsert(&self, conn: &mut PgConn) -> DbResult<CustomerPaymentMethodRow> {
        use crate::schema::customer_payment_method::dsl::{
            connection_id, customer_payment_method, external_payment_method_id,
        };
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(customer_payment_method)
            .values(self)
            .on_conflict((connection_id, external_payment_method_id))
            .do_update()
            .set(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting customer to connector")
            .into_db_result()
    }

    pub async fn insert_if_not_exist(
        &self,
        conn: &mut PgConn,
    ) -> DbResult<CustomerPaymentMethodRow> {
        use crate::schema::customer_payment_method::dsl::{
            connection_id, customer_payment_method, external_payment_method_id,
        };
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(customer_payment_method)
            .values(self)
            .on_conflict((connection_id, external_payment_method_id))
            .do_nothing();
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting customer to connector")
            .into_db_result()
    }
}
