use crate::errors::IntoDbResult;
use chrono::NaiveDate;

use crate::subscriptions::{SubscriptionForDisplayRow, SubscriptionRow, SubscriptionRowNew};
use crate::{DbResult, PgConn};

use diesel::{
    ExpressionMethods, JoinOnDsl, NullableExpressionMethods, OptionalExtension, QueryDsl,
    SelectableHelper, debug_query,
};
use diesel_async::RunQueryDsl;

use crate::enums::ConnectorProviderEnum;
use crate::extend::connection_metadata;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use common_domain::ids::{
    BaseId, ConnectorId, CustomerId, CustomerPaymentMethodId, PlanId, SubscriptionId, TenantId,
};
use error_stack::ResultExt;

impl SubscriptionRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SubscriptionRow> {
        use crate::schema::subscription::dsl::subscription;

        let query = diesel::insert_into(subscription).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting subscription")
            .into_db_result()
    }
}

impl SubscriptionRow {
    pub async fn insert_subscription_batch(
        conn: &mut PgConn,
        batch: Vec<&SubscriptionRowNew>,
    ) -> DbResult<Vec<SubscriptionRow>> {
        use crate::schema::subscription::dsl::subscription;

        let query = diesel::insert_into(subscription).values(batch);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while inserting subscription batch")
            .into_db_result()
    }

    pub async fn get_subscription_by_id(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<SubscriptionForDisplayRow> {
        use crate::schema::subscription::dsl::{id, subscription, tenant_id};

        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;

        let query = subscription
            .filter(id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .inner_join(crate::schema::customer::table)
            .inner_join(
                pv_dsl::plan_version.inner_join(p_dsl::plan.on(p_dsl::id.eq(pv_dsl::plan_id))),
            )
            .select(SubscriptionForDisplayRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result::<SubscriptionForDisplayRow>(conn)
            .await
            .attach("Error while fetching subscription by ID")
            .into_db_result()
    }

    pub async fn get_subscription_period_by_id(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<(NaiveDate, Option<NaiveDate>)> {
        use crate::schema::subscription::dsl::{
            current_period_end, current_period_start, id, subscription, tenant_id,
        };

        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;

        let query = subscription
            .filter(id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .inner_join(crate::schema::customer::table)
            .inner_join(
                pv_dsl::plan_version.inner_join(p_dsl::plan.on(p_dsl::id.eq(pv_dsl::plan_id))),
            )
            .select((current_period_start, current_period_end));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result::<(NaiveDate, Option<NaiveDate>)>(conn)
            .await
            .attach("Error while fetching subscription period by ID")
            .into_db_result()
    }

    pub async fn get_subscription_payment_method_by_id(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<Option<CustomerPaymentMethodId>> {
        use crate::schema::subscription::dsl::{id, payment_method, subscription, tenant_id};

        let query = subscription
            .filter(id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .select(payment_method);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result::<Option<CustomerPaymentMethodId>>(conn)
            .await
            .attach("Error while fetching subscription by ID")
            .into_db_result()
    }

    pub async fn list_subscriptions_by_ids(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_ids: &[SubscriptionId],
    ) -> DbResult<Vec<SubscriptionForDisplayRow>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::subscription::dsl::{id, subscription, tenant_id};

        let query = subscription
            .filter(id.eq_any(subscription_ids))
            .filter(tenant_id.eq(tenant_id_param))
            .inner_join(crate::schema::customer::table)
            .inner_join(
                pv_dsl::plan_version.inner_join(p_dsl::plan.on(p_dsl::id.eq(pv_dsl::plan_id))),
            )
            .order(id.desc())
            .select(SubscriptionForDisplayRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while fetching subscriptions by IDs")
            .into_db_result()
    }

    pub async fn list_by_customer_ids(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_ids: &[CustomerId],
    ) -> DbResult<Vec<SubscriptionForDisplayRow>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::subscription::dsl as s_dsl;

        let query = s_dsl::subscription
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(s_dsl::customer_id.eq_any(customer_ids))
            .inner_join(crate::schema::customer::table)
            .inner_join(
                pv_dsl::plan_version.inner_join(p_dsl::plan.on(p_dsl::id.eq(pv_dsl::plan_id))),
            )
            .select(SubscriptionForDisplayRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while fetching subscriptions by customer IDs")
            .into_db_result()
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        ids: &[SubscriptionId],
    ) -> DbResult<Vec<SubscriptionForDisplayRow>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::subscription::dsl as s_dsl;

        let query = s_dsl::subscription
            .filter(s_dsl::id.eq_any(ids))
            .inner_join(crate::schema::customer::table)
            .inner_join(
                pv_dsl::plan_version.inner_join(p_dsl::plan.on(p_dsl::id.eq(pv_dsl::plan_id))),
            )
            .select(SubscriptionForDisplayRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while fetching subscriptions by IDs")
            .into_db_result()
    }

    pub async fn get_subscription_id_by_invoice_id(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        invoice_id: &uuid::Uuid,
    ) -> DbResult<Option<SubscriptionId>> {
        use crate::schema::invoice::dsl as i_dsl;
        use crate::schema::subscription::dsl as s_dsl;

        let query = i_dsl::invoice
            .filter(i_dsl::id.eq(invoice_id))
            .filter(i_dsl::tenant_id.eq(tenant_id_param))
            .inner_join(s_dsl::subscription.on(s_dsl::id.nullable().eq(i_dsl::subscription_id)))
            .select(s_dsl::id);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result::<SubscriptionId>(conn)
            .await
            .optional()
            .attach("Error while fetching subscription by invoice ID")
            .into_db_result()
    }

    pub async fn list_subscriptions(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        customer_id_opt: Option<CustomerId>,
        plan_id_param_opt: Option<PlanId>,
        status_opt: Option<Vec<crate::enums::SubscriptionStatusEnum>>,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<SubscriptionForDisplayRow>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::subscription::dsl::{customer_id, id, status, subscription, tenant_id};

        let mut query = subscription
            .filter(tenant_id.eq(tenant_id_param))
            .inner_join(crate::schema::customer::table)
            .inner_join(
                pv_dsl::plan_version.inner_join(p_dsl::plan.on(p_dsl::id.eq(pv_dsl::plan_id))),
            )
            .into_boxed();

        if let Some(customer_id_param) = customer_id_opt {
            query = query.filter(customer_id.eq(customer_id_param));
        }

        if let Some(plan_id_param) = plan_id_param_opt {
            query = query.filter(p_dsl::id.eq(plan_id_param));
        }

        if let Some(status_param) = status_opt {
            query = query.filter(status.eq_any(status_param));
        }

        query = query.order(id.desc());

        let paginated_query = query
            .select(SubscriptionForDisplayRow::as_select())
            .paginate(pagination);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&paginated_query));

        paginated_query
            .load_and_count_pages::<SubscriptionForDisplayRow>(conn)
            .await
            .attach("Error while fetching subscriptions")
            .into_db_result()
    }

    pub async fn upsert_conn_meta(
        conn: &mut PgConn,
        provider: ConnectorProviderEnum,
        subscription_id: SubscriptionId,
        connector_id: ConnectorId,
        external_id: &str,
        external_company_id: &str,
    ) -> DbResult<()> {
        connection_metadata::upsert(
            conn,
            "subscription",
            provider.as_meta_key(),
            subscription_id.as_uuid(),
            connector_id,
            external_id,
            external_company_id,
        )
        .await
        .map(|_| ())
    }
}
