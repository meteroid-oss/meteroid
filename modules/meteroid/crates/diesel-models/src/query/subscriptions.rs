use crate::errors::IntoDbResult;
use chrono::NaiveDate;

use crate::subscriptions::{
    CancelSubscriptionParams, SubscriptionForDisplayRow, SubscriptionInvoiceCandidateRow,
    SubscriptionRow, SubscriptionRowNew,
};
use crate::{DbResult, PgConn};

use diesel::{
    BoolExpressionMethods, ExpressionMethods, JoinOnDsl, NullableExpressionMethods,
    OptionalExtension, QueryDsl, SelectableHelper, debug_query,
};
use diesel_async::RunQueryDsl;

use crate::enums::{ConnectorProviderEnum, InvoiceType};
use crate::extend::connection_metadata;
use crate::extend::cursor_pagination::{
    CursorPaginate, CursorPaginatedVec, CursorPaginationRequest,
};
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use common_domain::ids::{BaseId, ConnectorId, CustomerId, PlanId, SubscriptionId, TenantId};
use error_stack::ResultExt;
use uuid::Uuid;

impl SubscriptionRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SubscriptionRow> {
        use crate::schema::subscription::dsl::*;

        let query = diesel::insert_into(subscription).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting subscription")
            .into_db_result()
    }
}

impl SubscriptionRow {
    pub async fn insert_subscription_batch(
        conn: &mut PgConn,
        batch: Vec<&SubscriptionRowNew>,
    ) -> DbResult<Vec<SubscriptionRow>> {
        use crate::schema::subscription::dsl::*;

        let query = diesel::insert_into(subscription).values(batch);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting subscription batch")
            .into_db_result()
    }

    pub async fn get_subscription_by_id(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<SubscriptionForDisplayRow> {
        use crate::schema::subscription::dsl::*;

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
            .attach_printable("Error while fetching subscription by ID")
            .into_db_result()
    }

    pub async fn list_subscriptions_by_ids(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_ids: &[SubscriptionId],
    ) -> DbResult<Vec<SubscriptionForDisplayRow>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::subscription::dsl::*;

        let query = subscription
            .filter(id.eq_any(subscription_ids))
            .filter(tenant_id.eq(tenant_id_param))
            .inner_join(crate::schema::customer::table)
            .inner_join(
                pv_dsl::plan_version.inner_join(p_dsl::plan.on(p_dsl::id.eq(pv_dsl::plan_id))),
            )
            .select(SubscriptionForDisplayRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching subscriptions by IDs")
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
            .attach_printable("Error while fetching subscriptions by customer IDs")
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
            .attach_printable("Error while fetching subscriptions by IDs")
            .into_db_result()
    }

    pub async fn cancel_subscription(
        conn: &mut PgConn,
        params: CancelSubscriptionParams,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl::*;

        let query = diesel::update(subscription)
            .filter(id.eq(params.subscription_id))
            .filter(tenant_id.eq(params.tenant_id))
            .filter(canceled_at.is_null())
            .set((
                end_date.eq(params.billing_end_date),
                canceled_at.eq(params.canceled_at),
                cancellation_reason.eq(params.reason),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while canceling subscription")
            .into_db_result()?;

        Ok(())
    }

    pub async fn activate_subscription(
        conn: &mut PgConn,
        id: &SubscriptionId,
        tenant_id: &TenantId,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl as s_dsl;

        let query = diesel::update(s_dsl::subscription)
            .filter(s_dsl::id.eq(id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(s_dsl::activated_at.is_null())
            .set(s_dsl::activated_at.eq(chrono::Utc::now().naive_utc()));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while activating subscription")
            .into_db_result()?;

        Ok(())
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
            .attach_printable("Error while fetching subscription by invoice ID")
            .into_db_result()
    }

    pub async fn list_subscriptions(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        customer_id_opt: Option<CustomerId>,
        plan_id_param_opt: Option<PlanId>,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<SubscriptionForDisplayRow>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::subscription::dsl::*;

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

        //
        //
        //
        // query = query
        //     .inner_join(crate::schema::customer::table)
        //     .inner_join(crate::schema::plan_version::table.inner_join(crate::schema::plan::table));

        let paginated_query = query
            .select(SubscriptionForDisplayRow::as_select())
            .paginate(pagination);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&paginated_query));

        paginated_query
            .load_and_count_pages::<SubscriptionForDisplayRow>(conn)
            .await
            .attach_printable("Error while fetching subscriptions")
            .into_db_result()
    }

    pub async fn list_subscription_to_invoice_candidates(
        conn: &mut PgConn,
        input_date_param: NaiveDate,
        pagination: CursorPaginationRequest,
    ) -> DbResult<CursorPaginatedVec<SubscriptionInvoiceCandidateRow>> {
        use crate::schema::invoice::dsl as i_dsl;

        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_component::dsl as sc_dsl;

        let query = s_dsl::subscription
            // only if not already ended
            .filter(
                s_dsl::end_date
                    .is_null()
                    .or(s_dsl::end_date.gt(input_date_param)),
            )
            // only if started. lt => we consider that initial invoice was already created TODO start_date or billing_start_date?
            .filter(s_dsl::billing_start_date.lt(input_date_param))
            // only if no future recurring invoice exist.
            // (requires a single recurring invoice in parallel. For now, this is true)
            .left_join(
                i_dsl::invoice.on(s_dsl::id
                    .nullable()
                    .eq(i_dsl::subscription_id)
                    .and(i_dsl::invoice_type.eq(InvoiceType::Recurring))
                    .and(i_dsl::invoice_date.gt(input_date_param))),
            )
            .filter(i_dsl::id.is_null())
            .inner_join(
                pv_dsl::plan_version.inner_join(p_dsl::plan.on(p_dsl::id.eq(pv_dsl::plan_id))),
            )
            .left_join(sc_dsl::subscription_component)
            .inner_join(c_dsl::customer.on(c_dsl::id.eq(s_dsl::customer_id)))
            // only if customer is not archived
            .filter(c_dsl::archived_at.is_null())
            .select(SubscriptionInvoiceCandidateRow::as_select())
            .cursor_paginate(pagination, "id");

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load_and_get_next_cursor(conn, |a| a.subscription.id.as_uuid())
            .await
            .attach_printable("Error while fetching subscriptions to invoice")
            .into_db_result()
    }

    pub async fn update_subscription_mrr_delta(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        mrr_cents_delta: i64,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl::*;

        let query = diesel::update(subscription)
            .filter(id.eq(subscription_id))
            .set(mrr_cents.eq(mrr_cents + mrr_cents_delta));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while updating subscription MRR")
            .into_db_result()?;

        Ok(())
    }

    pub async fn lock_subscription_for_update(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl::*;

        let query = subscription
            .for_update()
            .select(id)
            .filter(id.eq(subscription_id_param));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let _res: Uuid = query
            .get_result(conn)
            .await
            .attach_printable("Error while locking subscription for update")
            .into_db_result()?;

        Ok(())
    }

    pub async fn upsert_conn_meta(
        conn: &mut PgConn,
        provider: ConnectorProviderEnum,
        subscription_id: SubscriptionId,
        connector_id: ConnectorId,
        external_id: &str,
    ) -> DbResult<()> {
        connection_metadata::upsert(
            conn,
            "subscription",
            provider.as_meta_key(),
            subscription_id.as_uuid(),
            connector_id,
            external_id,
        )
        .await
        .map(|_| ())
    }
}
