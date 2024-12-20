use crate::errors::IntoDbResult;
use chrono::NaiveDate;

use crate::subscriptions::{
    CancelSubscriptionParams, SubscriptionForDisplayRow, SubscriptionInvoiceCandidateRow,
    SubscriptionRow, SubscriptionRowNew,
};
use crate::{DbResult, PgConn};

use diesel::{
    debug_query, BoolExpressionMethods, ExpressionMethods, JoinOnDsl, NullableExpressionMethods,
    OptionalExtension, QueryDsl, SelectableHelper,
};
use diesel_async::RunQueryDsl;

use crate::enums::InvoiceType;
use crate::extend::cursor_pagination::{
    CursorPaginate, CursorPaginatedVec, CursorPaginationRequest,
};
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::query::IdentityDb;
use error_stack::ResultExt;
use uuid::Uuid;

impl SubscriptionRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SubscriptionRow> {
        use crate::schema::subscription::dsl::*;

        let query = diesel::insert_into(subscription).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting subscription batch")
            .into_db_result()
    }

    pub async fn get_subscription_by_id(
        conn: &mut PgConn,
        tenant_id_param: &uuid::Uuid,
        subscription_id_param: IdentityDb,
    ) -> DbResult<SubscriptionForDisplayRow> {
        use crate::schema::subscription::dsl::*;

        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;

        let mut query = subscription
            .filter(tenant_id.eq(tenant_id_param))
            .inner_join(crate::schema::customer::table)
            .inner_join(
                pv_dsl::plan_version.inner_join(p_dsl::plan.on(p_dsl::id.eq(pv_dsl::plan_id))),
            )
            .select(SubscriptionForDisplayRow::as_select())
            .into_boxed();

        match subscription_id_param {
            IdentityDb::UUID(id_param) => {
                query = query.filter(id.eq(id_param));
            }
            IdentityDb::LOCAL(local_id_param) => {
                query = query.filter(local_id.eq(local_id_param));
            }
        }

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result::<SubscriptionForDisplayRow>(conn)
            .await
            .attach_printable("Error while fetching subscription by ID")
            .into_db_result()
    }

    pub async fn list_subscriptions_by_ids(
        conn: &mut PgConn,
        tenant_id_param: &uuid::Uuid,
        subscription_ids: &[uuid::Uuid],
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

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
                billing_end_date.eq(params.billing_end_date),
                canceled_at.eq(params.canceled_at),
                cancellation_reason.eq(params.reason),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while canceling subscription")
            .into_db_result()?;

        Ok(())
    }

    pub async fn activate_subscription(
        conn: &mut PgConn,
        id: Uuid,
        tenant_id: Uuid,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl as s_dsl;

        let query = diesel::update(s_dsl::subscription)
            .filter(s_dsl::id.eq(id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(s_dsl::activated_at.is_null())
            .set(s_dsl::activated_at.eq(chrono::Utc::now().naive_utc()));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while activating subscription")
            .into_db_result()?;

        Ok(())
    }

    pub async fn get_subscription_id_by_invoice_id(
        conn: &mut PgConn,
        tenant_id_param: &uuid::Uuid,
        invoice_id: &uuid::Uuid,
    ) -> DbResult<Option<uuid::Uuid>> {
        use crate::schema::invoice::dsl as i_dsl;
        use crate::schema::subscription::dsl as s_dsl;

        let query = i_dsl::invoice
            .filter(i_dsl::id.eq(invoice_id))
            .filter(i_dsl::tenant_id.eq(tenant_id_param))
            .inner_join(s_dsl::subscription.on(s_dsl::id.nullable().eq(i_dsl::subscription_id)))
            .select(s_dsl::id);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result::<uuid::Uuid>(conn)
            .await
            .optional()
            .attach_printable("Error while fetching subscription by invoice ID")
            .into_db_result()
    }

    pub async fn list_subscriptions(
        conn: &mut PgConn,
        tenant_id_param: uuid::Uuid,
        customer_id_opt: Option<IdentityDb>,
        plan_id_param_opt: Option<IdentityDb>,
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
            match customer_id_param {
                IdentityDb::UUID(customer_id_param) => {
                    query = query.filter(customer_id.eq(customer_id_param));
                }
                IdentityDb::LOCAL(customer_local_id) => {
                    query = query.filter(crate::schema::customer::local_id.eq(customer_local_id));
                }
            }
        }

        if let Some(plan_id_param) = plan_id_param_opt {
            match plan_id_param {
                IdentityDb::UUID(plan_id) => {
                    query = query.filter(p_dsl::id.eq(plan_id));
                }
                IdentityDb::LOCAL(plan_local_id) => {
                    query = query.filter(p_dsl::local_id.eq(plan_local_id));
                }
            }
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

        log::debug!(
            "{}",
            debug_query::<diesel::pg::Pg, _>(&paginated_query).to_string()
        );

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

        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_component::dsl as sc_dsl;

        let query = s_dsl::subscription
            // only if not already ended
            .filter(
                s_dsl::billing_end_date
                    .is_null()
                    .or(s_dsl::billing_end_date.gt(input_date_param)),
            )
            // only if started. lt => we consider that initial invoice was already created
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
            .select(SubscriptionInvoiceCandidateRow::as_select())
            .cursor_paginate(pagination, "id");

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .load_and_get_next_cursor(conn, |a| a.subscription.id)
            .await
            .attach_printable("Error while fetching subscriptions to invoice")
            .into_db_result()
    }

    pub async fn update_subscription_mrr_delta(
        conn: &mut PgConn,
        subscription_id: uuid::Uuid,
        mrr_cents_delta: i64,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl::*;

        let query = diesel::update(subscription)
            .filter(id.eq(subscription_id))
            .set(mrr_cents.eq(mrr_cents + mrr_cents_delta));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while updating subscription MRR")
            .into_db_result()?;

        Ok(())
    }

    pub async fn lock_subscription_for_update(
        conn: &mut PgConn,
        subscription_id_param: uuid::Uuid,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl::*;

        let query = subscription
            .for_update()
            .select(id)
            .filter(id.eq(subscription_id_param));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        let _res: Uuid = query
            .get_result(conn)
            .await
            .attach_printable("Error while locking subscription for update")
            .into_db_result()?;

        Ok(())
    }
}
