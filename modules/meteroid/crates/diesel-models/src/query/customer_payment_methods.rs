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
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use error_stack::ResultExt;

/// How long after being saved for a payment a method still counts as newly saved.
pub const SAVED_FOR_PAYMENT_WINDOW_HOURS: i32 = 24;

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
        crate::debug_sql(&query);

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
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while finding customer payment method by id")
            .into_db_result()
    }

    /// Resolve a method by the provider's payment-method id (mandate id),
    /// scoped to the tenant. Used by detach webhooks (mandate cancelled).
    pub async fn get_by_external_id(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        external_payment_method_id_param: &str,
    ) -> DbResult<Option<CustomerPaymentMethodRow>> {
        use crate::schema::customer_payment_method::dsl as cpm_dsl;
        use diesel_async::RunQueryDsl;

        let query = cpm_dsl::customer_payment_method
            .filter(cpm_dsl::external_payment_method_id.eq(external_payment_method_id_param))
            .filter(cpm_dsl::tenant_id.eq(tenant_id_param))
            .select(CustomerPaymentMethodRow::as_select());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding customer payment method by external id")
            .into_db_result()
    }

    /// Soft-archive a method (kept for historical transactions' FK). Only sets
    /// `archived_at` when still null, so redeliveries preserve the first stamp.
    pub async fn archive(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        id_param: &CustomerPaymentMethodId,
    ) -> DbResult<usize> {
        use crate::schema::customer_payment_method::dsl as cpm_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(cpm_dsl::customer_payment_method)
            .filter(cpm_dsl::id.eq(id_param))
            .filter(cpm_dsl::tenant_id.eq(tenant_id_param))
            .filter(cpm_dsl::archived_at.is_null())
            .set((
                cpm_dsl::archived_at.eq(common_clock::now().naive_utc()),
                cpm_dsl::updated_at.eq(common_clock::now().naive_utc()),
            ));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while archiving customer payment method")
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
        crate::debug_sql(&query);

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
        crate::debug_sql(&query);

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
        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while finding customer payment methods by connection id")
            .into_db_result()
    }

    /// The customer holds a card that can be charged: not archived, on an active connector.
    pub async fn has_chargeable_card(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        customer_id_param: &CustomerId,
    ) -> DbResult<bool> {
        use crate::enums::{ConnectorStatusEnum, PaymentMethodTypeEnum};
        use crate::schema::connector::dsl as k_dsl;
        use crate::schema::customer_connection::dsl as cc_dsl;
        use crate::schema::customer_payment_method::dsl as cpm_dsl;
        use diesel::JoinOnDsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::select(diesel::dsl::exists(
            cpm_dsl::customer_payment_method
                .inner_join(cc_dsl::customer_connection.on(cc_dsl::id.eq(cpm_dsl::connection_id)))
                .inner_join(k_dsl::connector.on(k_dsl::id.eq(cc_dsl::connector_id)))
                .filter(cpm_dsl::tenant_id.eq(tenant_id_param))
                .filter(cpm_dsl::customer_id.eq(customer_id_param))
                .filter(cpm_dsl::archived_at.is_null())
                .filter(cpm_dsl::payment_method_type.eq(PaymentMethodTypeEnum::Card))
                .filter(k_dsl::status.eq(ConnectorStatusEnum::Active)),
        ));
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while checking the customer's chargeable cards")
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
        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while finding customer payment methods by connection id")
            .into_db_result()
    }

    /// Refresh card brand/last4/expiry for a stored method, keyed by the
    /// provider's payment-method id (used by the `payment_method.updated`
    /// webhook). Returns the number of rows updated.
    pub async fn update_card_details(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        external_payment_method_id_param: &str,
        brand: Option<String>,
        last4: Option<String>,
        exp_month: Option<i32>,
        exp_year: Option<i32>,
        refreshed: bool,
    ) -> DbResult<usize> {
        use crate::schema::customer_payment_method::dsl as cpm;
        use diesel_async::RunQueryDsl;

        let target = cpm::customer_payment_method
            .filter(cpm::tenant_id.eq(tenant_uid))
            .filter(cpm::external_payment_method_id.eq(external_payment_method_id_param));
        let details = (
            cpm::card_brand.eq(brand),
            cpm::card_last4.eq(last4),
            cpm::card_exp_month.eq(exp_month),
            cpm::card_exp_year.eq(exp_year),
            cpm::updated_at.eq(common_clock::now().naive_utc()),
        );

        // Only a new number or expiry stamps the refresh; a brand-only change keeps the last one.
        let result = if refreshed {
            let query = diesel::update(target).set((
                details,
                cpm::card_refreshed_at.eq(Some(common_clock::now())),
            ));
            crate::debug_sql(&query);
            query.execute(conn).await
        } else {
            let query = diesel::update(target).set(details);
            crate::debug_sql(&query);
            query.execute(conn).await
        };
        result
            .attach("Error while updating payment method card details")
            .into_db_result()
    }

    /// Marks (again, if saved anew) a method the customer saved to pay with right away, unless it
    /// already paid once.
    pub async fn mark_saved_for_payment(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        payment_method_id: CustomerPaymentMethodId,
    ) -> DbResult<usize> {
        use crate::enums::PaymentStatusEnum;
        use crate::schema::customer_payment_method::dsl as cpm;
        use crate::schema::payment_transaction::dsl as pt;
        use diesel::dsl::{exists, not};
        use diesel_async::RunQueryDsl;

        let paid = pt::payment_transaction
            .filter(pt::tenant_id.eq(tenant_uid))
            .filter(pt::payment_method_id.eq(payment_method_id))
            .filter(pt::status.eq_any([PaymentStatusEnum::Settled, PaymentStatusEnum::Refunded]));
        let query = diesel::update(cpm::customer_payment_method)
            .filter(cpm::tenant_id.eq(tenant_uid))
            .filter(cpm::id.eq(payment_method_id))
            .filter(not(exists(paid)))
            .set(cpm::saved_for_payment_at.eq(common_clock::now().naive_utc()));
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while marking a payment method saved for a payment")
            .into_db_result()
    }

    /// Clears the mark a settled payment consumes; true when set at most the window's hours before
    /// the payment started, not settled: a direct debit settles days later, while a card saved for
    /// a payment that failed weeks ago is an older card.
    pub async fn take_saved_for_payment(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        payment_method_id: CustomerPaymentMethodId,
        payment_started_at: chrono::DateTime<chrono::Utc>,
    ) -> DbResult<bool> {
        use crate::schema::customer_payment_method::dsl as cpm;
        use diesel_async::RunQueryDsl;

        let target = cpm::customer_payment_method
            .filter(cpm::tenant_id.eq(tenant_uid))
            .filter(cpm::id.eq(payment_method_id));
        let fresh = diesel::update(target)
            .filter(cpm::saved_for_payment_at.gt(payment_started_at
                - chrono::Duration::hours(i64::from(SAVED_FOR_PAYMENT_WINDOW_HOURS))))
            .set(cpm::saved_for_payment_at.eq(None::<chrono::DateTime<chrono::Utc>>));
        crate::debug_sql(&fresh);
        let fresh = fresh
            .execute(conn)
            .await
            .attach("Error while clearing a payment method's saved-for-payment mark")
            .into_db_result()?
            > 0;

        let stale = diesel::update(target)
            .filter(cpm::saved_for_payment_at.is_not_null())
            .set(cpm::saved_for_payment_at.eq(None::<chrono::DateTime<chrono::Utc>>));
        crate::debug_sql(&stale);
        stale
            .execute(conn)
            .await
            .attach("Error while clearing a stale saved-for-payment mark")
            .into_db_result()?;
        Ok(fresh)
    }
}

impl CustomerPaymentMethodRowNew {
    /// Insert, or refresh the row with the same provider id.
    pub async fn upsert(&self, conn: &mut PgConn) -> DbResult<CustomerPaymentMethodRow> {
        use crate::schema::customer_payment_method::dsl as cpm_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(cpm_dsl::customer_payment_method)
            .values(self)
            .on_conflict((cpm_dsl::connection_id, cpm_dsl::external_payment_method_id))
            .do_update()
            .set((
                self,
                cpm_dsl::updated_at.eq(common_clock::now().naive_utc()),
            ));
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while upserting customer payment method")
            .into_db_result()
    }

    /// Upserts on the active `(connection, type, fingerprint)` row, adopting the new provider id.
    /// The partial unique index makes concurrent inserts converge. Requires `fingerprint`.
    pub async fn upsert_by_fingerprint(
        &self,
        conn: &mut PgConn,
    ) -> DbResult<CustomerPaymentMethodRow> {
        use crate::schema::customer_payment_method::dsl as cpm_dsl;
        use diesel::upsert::DecoratableTarget;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(cpm_dsl::customer_payment_method)
            .values(self)
            .on_conflict((
                cpm_dsl::connection_id,
                cpm_dsl::payment_method_type,
                cpm_dsl::fingerprint,
            ))
            .filter_target(cpm_dsl::archived_at.is_null())
            .do_update()
            .set((
                self,
                cpm_dsl::updated_at.eq(common_clock::now().naive_utc()),
            ));
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while upserting customer payment method by fingerprint")
            .into_db_result()
    }
}

/// A customer's current card that an automatically charged subscription uses, about to expire.
#[derive(Debug, diesel::QueryableByName)]
pub struct ExpiringCardRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: uuid::Uuid,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub tenant_id: uuid::Uuid,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub customer_id: uuid::Uuid,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub card_brand: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub card_last4: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Int4)]
    pub card_exp_month: i32,
    #[diesel(sql_type = diesel::sql_types::Int4)]
    pub card_exp_year: i32,
}

/// Cards valid through a day in `from..=to`, still unnotified for that expiry, that are their
/// customer's current method and that a running subscription charges automatically online, where
/// the customer's invoicing entity sends the expiring-card email.
pub async fn list_expiring_charged_cards(
    conn: &mut PgConn,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
    limit: i64,
) -> DbResult<Vec<ExpiringCardRow>> {
    use diesel::sql_types::{Date, Int8};
    use diesel_async::RunQueryDsl;

    diesel::sql_query(
        "SELECT DISTINCT cpm.id, cpm.tenant_id, cpm.customer_id, cpm.card_brand, cpm.card_last4,
                cpm.card_exp_month, cpm.card_exp_year
         FROM customer_payment_method cpm
         JOIN customer c ON c.id = cpm.customer_id AND c.current_payment_method_id = cpm.id
         JOIN invoicing_entity_email_settings es
           ON es.invoicing_entity_id = c.invoicing_entity_id
          AND NOT ('payment_method_expiring' = ANY (es.disabled_kinds))
         JOIN subscription s ON s.customer_id = c.id AND s.tenant_id = c.tenant_id
         WHERE cpm.archived_at IS NULL
           AND c.archived_at IS NULL
           AND cpm.payment_method_type = 'CARD'
           AND cpm.card_exp_month BETWEEN 1 AND 12
           AND cpm.card_exp_year IS NOT NULL
           AND (make_date(cpm.card_exp_year, cpm.card_exp_month, 1) + interval '1 month'
                - interval '1 day')::date BETWEEN $1 AND $2
           AND s.status IN ('ACTIVE', 'TRIAL_ACTIVE')
           AND s.charge_automatically
           AND (s.payment_methods_config IS NULL OR s.payment_methods_config ->> 'type' = 'online')
           AND NOT EXISTS (
               SELECT 1 FROM payment_method_expiry_notice n
               WHERE n.payment_method_id = cpm.id
                 AND n.exp_year = cpm.card_exp_year
                 AND n.exp_month = cpm.card_exp_month)
         ORDER BY cpm.id
         LIMIT $3",
    )
    .bind::<Date, _>(from)
    .bind::<Date, _>(to)
    .bind::<Int8, _>(limit)
    .load(conn)
    .await
    .attach("Error while listing expiring cards")
    .into_db_result()
}

/// Records that the card's customer was told of this expiry: false when it already was.
pub async fn record_expiry_notice(
    conn: &mut PgConn,
    tenant_id: TenantId,
    payment_method_id: CustomerPaymentMethodId,
    exp_year: i32,
    exp_month: i32,
) -> DbResult<bool> {
    use diesel::sql_types::{Int4, Uuid};
    use diesel_async::RunQueryDsl;

    diesel::sql_query(
        "INSERT INTO payment_method_expiry_notice (payment_method_id, tenant_id, exp_year, exp_month)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT DO NOTHING",
    )
    .bind::<Uuid, _>(payment_method_id)
    .bind::<Uuid, _>(tenant_id)
    .bind::<Int4, _>(exp_year)
    .bind::<Int4, _>(exp_month)
    .execute(conn)
    .await
    .map(|inserted| inserted > 0)
    .attach("Error while recording a card expiry notice")
    .into_db_result()
}
