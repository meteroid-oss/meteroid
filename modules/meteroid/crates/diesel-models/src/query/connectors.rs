use crate::connectors::{ConnectorRow, ConnectorRowNew, ConnectorRowPatch};
use crate::enums::ConnectorStatusEnum;
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};

use common_domain::ids::{ConnectorId, TenantId};
use diesel::prelude::{ExpressionMethods, OptionalExtension, QueryDsl};
use error_stack::ResultExt;

impl ConnectorRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<ConnectorRow> {
        use crate::schema::connector::dsl::connector;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(connector).values(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting connector")
            .into_db_result()
    }
}

impl ConnectorRow {
    /// Hard delete, for drafts only: nothing references a draft.
    pub async fn delete_draft(
        conn: &mut PgConn,
        id: ConnectorId,
        tenant_uid: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::connector::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(
            c_dsl::connector
                .filter(c_dsl::id.eq(id))
                .filter(c_dsl::tenant_id.eq(tenant_uid))
                .filter(c_dsl::status.eq(ConnectorStatusEnum::Draft)),
        );

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting connector draft")
            .into_db_result()
    }

    /// Any status, so payment history can still name a deleted connector.
    pub async fn get_connector_by_id(
        conn: &mut PgConn,
        id: ConnectorId,
        tenant_uid: TenantId,
    ) -> DbResult<ConnectorRow> {
        use crate::schema::connector::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::connector
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_uid));

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding connector")
            .into_db_result()
    }

    /// Any tenant, any status.
    pub async fn get_connector_by_id_unscoped(
        conn: &mut PgConn,
        id: ConnectorId,
    ) -> DbResult<ConnectorRow> {
        use crate::schema::connector::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::connector.filter(c_dsl::id.eq(id));

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding connector")
            .into_db_result()
    }

    /// Skips deleted connectors: a deleted alias may have been reused.
    pub async fn get_connector_by_alias(
        conn: &mut PgConn,
        connector_alias: String,
        tenant_uid: TenantId,
    ) -> DbResult<ConnectorRow> {
        use crate::schema::connector::dsl::{alias, connector, status, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = connector
            .filter(alias.eq(connector_alias))
            .filter(tenant_id.eq(tenant_uid))
            .filter(status.ne(ConnectorStatusEnum::Deleted));

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding connector")
            .into_db_result()
    }

    /// Excludes deleted connectors.
    pub async fn list_connectors(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        connector_type_filter: Option<crate::enums::ConnectorTypeEnum>,
        provider_filter: Option<crate::enums::ConnectorProviderEnum>,
    ) -> DbResult<Vec<ConnectorRow>> {
        use crate::schema::connector::dsl::{
            connector, connector_type, created_at, provider, status, tenant_id,
        };
        use diesel_async::RunQueryDsl;

        let mut query = connector
            .filter(tenant_id.eq(tenant_uid))
            .filter(status.ne(ConnectorStatusEnum::Deleted))
            .order(created_at.asc())
            .into_boxed();

        if let Some(ct) = connector_type_filter {
            query = query.filter(connector_type.eq(ct));
        }

        if let Some(cp) = provider_filter {
            query = query.filter(provider.eq(cp));
        }

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing connectors")
            .into_db_result()
    }

    /// Every non-deleted connector of one provider across all tenants (maintenance passes).
    pub async fn list_by_provider_unscoped(
        conn: &mut PgConn,
        provider_filter: crate::enums::ConnectorProviderEnum,
    ) -> DbResult<Vec<ConnectorRow>> {
        use crate::schema::connector::dsl::{connector, created_at, provider, status};
        use diesel_async::RunQueryDsl;

        let query = connector
            .filter(provider.eq(provider_filter))
            .filter(status.ne(ConnectorStatusEnum::Deleted))
            .order(created_at.asc());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing connectors by provider")
            .into_db_result()
    }

    /// The subset of `ids` that are active.
    pub async fn filter_active_ids(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        ids: &[ConnectorId],
    ) -> DbResult<Vec<ConnectorId>> {
        use crate::schema::connector::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::connector
            .filter(c_dsl::tenant_id.eq(tenant_uid))
            .filter(c_dsl::id.eq_any(ids))
            .filter(c_dsl::status.eq(ConnectorStatusEnum::Active))
            .select(c_dsl::id);

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while filtering active connectors")
            .into_db_result()
    }

    pub async fn set_status(
        conn: &mut PgConn,
        id: ConnectorId,
        tenant_uid: TenantId,
        new_status: ConnectorStatusEnum,
    ) -> DbResult<ConnectorRow> {
        use crate::schema::connector::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let now = common_domain::clock::now().naive_utc();
        let query = diesel::update(c_dsl::connector)
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_uid))
            .set((c_dsl::status.eq(new_status), c_dsl::updated_at.eq(now)));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while updating connector status")
            .into_db_result()
    }

    /// Soft delete: the row stays for payment history; its secrets do not.
    pub async fn soft_delete(
        conn: &mut PgConn,
        id: ConnectorId,
        tenant_uid: TenantId,
    ) -> DbResult<ConnectorRow> {
        use crate::schema::connector::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let now = common_domain::clock::now().naive_utc();
        let query = diesel::update(c_dsl::connector)
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_uid))
            .set((
                c_dsl::status.eq(ConnectorStatusEnum::Deleted),
                c_dsl::sensitive.eq(None::<String>),
                c_dsl::updated_at.eq(now),
            ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while deleting connector")
            .into_db_result()
    }

    /// Lock taken by paths that start a payment: `FOR KEY SHARE` conflicts with the `FOR UPDATE`
    /// of a delete or status change (`lock_by_id`), which then waits for the payment's
    /// transaction and sees what it created. It doesn't block plain updates of other columns, so
    /// the payment can still flag rejected credentials from another connection.
    pub async fn lock_key_shared_by_id(
        conn: &mut PgConn,
        id: ConnectorId,
        tenant_uid: TenantId,
    ) -> DbResult<ConnectorRow> {
        use crate::schema::connector::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::connector
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_uid))
            .for_key_share();

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while locking connector")
            .into_db_result()
    }

    /// Records (`Some`) or clears (`None`) a rejection of the stored credentials.
    pub async fn set_credentials_error(
        conn: &mut PgConn,
        id: ConnectorId,
        tenant_uid: TenantId,
        error: Option<String>,
    ) -> DbResult<usize> {
        use crate::schema::connector::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let at = error
            .as_ref()
            .map(|_| common_domain::clock::now().naive_utc());
        let query = diesel::update(c_dsl::connector)
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_uid))
            .set((
                c_dsl::credentials_error.eq(error),
                c_dsl::credentials_error_at.eq(at),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while recording connector credentials error")
            .into_db_result()
    }

    /// Row lock for status transitions, so a delete can't race an in-flight check.
    pub async fn lock_by_id(
        conn: &mut PgConn,
        id: ConnectorId,
        tenant_uid: TenantId,
    ) -> DbResult<ConnectorRow> {
        use crate::schema::connector::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::connector
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_uid))
            .for_update();

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while locking connector")
            .into_db_result()
    }

    /// `lock_by_id` without waiting: None while another transaction holds the connector (a
    /// payment starting through it holds KEY SHARE) or when it does not exist.
    pub async fn try_lock_by_id(
        conn: &mut PgConn,
        id: ConnectorId,
        tenant_uid: TenantId,
    ) -> DbResult<Option<ConnectorRow>> {
        use crate::schema::connector::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = c_dsl::connector
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_uid))
            .for_update()
            .skip_locked();

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while trying to lock connector")
            .into_db_result()
    }

    pub async fn usage(
        conn: &mut PgConn,
        id: ConnectorId,
        tenant_uid: TenantId,
    ) -> DbResult<ConnectorUsageRow> {
        Ok(ConnectorUsageRow {
            customers_with_saved_methods: Self::count_customers_with_saved_methods(
                conn, id, tenant_uid,
            )
            .await?,
            active_subscriptions: Self::count_active_subscriptions(conn, id, tenant_uid).await?,
            in_flight_transactions: Self::count_in_flight_transactions(conn, id, tenant_uid)
                .await?,
        })
    }

    async fn count_customers_with_saved_methods(
        conn: &mut PgConn,
        id: ConnectorId,
        tenant_uid: TenantId,
    ) -> DbResult<i64> {
        use crate::schema::customer_connection::dsl as cc_dsl;
        use crate::schema::customer_payment_method::dsl as pm_dsl;
        use diesel::AggregateExpressionMethods;
        use diesel::dsl::count;
        use diesel_async::RunQueryDsl;

        let query = pm_dsl::customer_payment_method
            .inner_join(cc_dsl::customer_connection)
            .filter(pm_dsl::tenant_id.eq(tenant_uid))
            .filter(cc_dsl::connector_id.eq(id))
            .filter(pm_dsl::archived_at.is_null())
            .select(count(pm_dsl::customer_id).aggregate_distinct());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while counting customers with saved methods")
            .into_db_result()
    }

    /// Subscriptions whose customer's invoicing entity routes a rail here and whose own config
    /// doesn't opt out of online payment.
    async fn count_active_subscriptions(
        conn: &mut PgConn,
        id: ConnectorId,
        tenant_uid: TenantId,
    ) -> DbResult<i64> {
        use crate::enums::SubscriptionStatusEnum;
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::invoicing_entity::dsl as ie_dsl;
        use crate::schema::subscription::dsl as s_dsl;
        use diesel::BoolExpressionMethods;
        use diesel::PgAnyJsonExpressionMethods;
        use diesel::dsl::count_star;
        use diesel_async::RunQueryDsl;

        let query = s_dsl::subscription
            .inner_join(c_dsl::customer.inner_join(ie_dsl::invoicing_entity))
            .filter(s_dsl::tenant_id.eq(tenant_uid))
            .filter(s_dsl::status.eq_any(SubscriptionStatusEnum::not_terminal()))
            .filter(
                ie_dsl::card_provider_id
                    .eq(id)
                    .or(ie_dsl::direct_debit_provider_id.eq(id)),
            )
            .filter(
                s_dsl::payment_methods_config
                    .is_null()
                    .or(s_dsl::payment_methods_config
                        .retrieve_as_text("type")
                        .eq("online")),
            )
            .select(count_star());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while counting active subscriptions")
            .into_db_result()
    }

    /// Pending or ready transactions that reach this connector through their payment method,
    /// their pending hosted intent, or, for hosted checkouts that record neither until
    /// fulfilled (GoCardless), the checkout customer's connection here.
    async fn count_in_flight_transactions(
        conn: &mut PgConn,
        id: ConnectorId,
        tenant_uid: TenantId,
    ) -> DbResult<i64> {
        use crate::enums::PaymentStatusEnum;
        use crate::schema::checkout_session::dsl as cs_dsl;
        use crate::schema::customer_connection::dsl as cc_dsl;
        use crate::schema::customer_payment_method::dsl as pm_dsl;
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel::AggregateExpressionMethods;
        use diesel::dsl::count;
        use diesel::{BoolExpressionMethods, JoinOnDsl, NullableExpressionMethods};
        use diesel_async::RunQueryDsl;

        let (pending_conn, hosted_conn) = diesel::alias!(
            crate::schema::customer_connection as pending_conn,
            crate::schema::customer_connection as hosted_conn
        );

        // `customer_connection` is unique per (customer, connector): no join multiplies rows.
        let query =
            pt_dsl::payment_transaction
                .left_join(pm_dsl::customer_payment_method.left_join(cc_dsl::customer_connection))
                .left_join(pending_conn.on(
                    pt_dsl::pending_connection_id.eq(pending_conn.field(cc_dsl::id).nullable()),
                ))
                .left_join(cs_dsl::checkout_session)
                .left_join(
                    hosted_conn.on(hosted_conn
                        .field(cc_dsl::customer_id)
                        .nullable()
                        .eq(cs_dsl::customer_id.nullable())
                        .and(hosted_conn.field(cc_dsl::connector_id).eq(id))),
                )
                .filter(pt_dsl::tenant_id.eq(tenant_uid))
                .filter(
                    pt_dsl::status.eq_any([PaymentStatusEnum::Pending, PaymentStatusEnum::Ready]),
                )
                .filter(
                    cc_dsl::connector_id
                        .nullable()
                        .eq(id)
                        .or(pending_conn.field(cc_dsl::connector_id).nullable().eq(id))
                        .or(pt_dsl::payment_method_id
                            .is_null()
                            .and(pt_dsl::pending_connection_id.is_null())
                            .and(hosted_conn.field(cc_dsl::id).nullable().is_not_null())),
                )
                .select(count(pt_dsl::id).aggregate_distinct());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while counting in-flight transactions")
            .into_db_result()
    }
}

#[derive(Debug)]
pub struct ConnectorUsageRow {
    pub customers_with_saved_methods: i64,
    pub active_subscriptions: i64,
    pub in_flight_transactions: i64,
}

impl ConnectorRowPatch {
    pub async fn patch(&self, conn: &mut PgConn, tenant_id: TenantId) -> DbResult<ConnectorRow> {
        use crate::schema::connector::dsl as c_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(c_dsl::connector)
            .filter(c_dsl::id.eq(self.id))
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .set(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while patching connector")
            .into_db_result()
    }
}
