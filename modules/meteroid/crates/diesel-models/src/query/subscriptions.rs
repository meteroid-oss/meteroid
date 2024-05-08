use crate::errors::IntoDbResult;

use crate::subscriptions::{
    CancelSubscriptionParams, Subscription, SubscriptionForDisplay, SubscriptionNew,
};
use crate::{DbResult, PgConn};

use diesel::{debug_query, ExpressionMethods, QueryDsl, SelectableHelper};

use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use error_stack::ResultExt;
use uuid::Uuid;

impl SubscriptionNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<Subscription> {
        use crate::schema::subscription::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(subscription).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting subscription")
            .into_db_result()
    }
}

impl Subscription {
    pub async fn insert_subscription_batch(
        conn: &mut PgConn,
        batch: Vec<&SubscriptionNew>,
    ) -> DbResult<Vec<Subscription>> {
        use crate::schema::subscription::dsl::*;
        use diesel_async::RunQueryDsl;

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
        subscription_id: &uuid::Uuid,
    ) -> DbResult<SubscriptionForDisplay> {
        use crate::schema::subscription::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = subscription
            .filter(id.eq(subscription_id))
            .filter(tenant_id.eq(tenant_id_param))
            .inner_join(crate::schema::customer::table)
            .inner_join(crate::schema::plan_version::table.inner_join(crate::schema::plan::table))
            .select(SubscriptionForDisplay::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result::<SubscriptionForDisplay>(conn)
            .await
            .attach_printable("Error while fetching subscription by ID")
            .into_db_result()
    }

    pub async fn get_subscriptions_by_ids(
        conn: &mut PgConn,
        tenant_id_param: &uuid::Uuid,
        subscription_ids: &[uuid::Uuid],
    ) -> DbResult<Vec<SubscriptionForDisplay>> {
        use crate::schema::subscription::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = subscription
            .filter(id.eq_any(subscription_ids))
            .filter(tenant_id.eq(tenant_id_param))
            .inner_join(crate::schema::customer::table)
            .inner_join(crate::schema::plan_version::table.inner_join(crate::schema::plan::table))
            .select(SubscriptionForDisplay::as_select());

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
        use diesel_async::RunQueryDsl;

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

    pub async fn list_subscriptions(
        conn: &mut PgConn,
        tenant_id_param: uuid::Uuid,
        customer_id_opt: Option<uuid::Uuid>,
        plan_id_param_opt: Option<uuid::Uuid>,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<SubscriptionForDisplay>> {
        use crate::schema::subscription::dsl::*;

        let mut query = subscription
            .filter(tenant_id.eq(tenant_id_param))
            .inner_join(crate::schema::customer::table)
            .inner_join(crate::schema::plan_version::table.inner_join(crate::schema::plan::table))
            .into_boxed();

        if let Some(customer_id_param) = customer_id_opt {
            query = query.filter(customer_id.eq(customer_id_param));
        }

        if let Some(plan_id_param) = plan_id_param_opt {
            query = query.filter(crate::schema::plan::id.eq(plan_id_param));
        }
        //
        //
        //
        // query = query
        //     .inner_join(crate::schema::customer::table)
        //     .inner_join(crate::schema::plan_version::table.inner_join(crate::schema::plan::table));

        let paginated_query = query
            .select(SubscriptionForDisplay::as_select())
            .paginate(pagination);

        log::debug!(
            "{}",
            debug_query::<diesel::pg::Pg, _>(&paginated_query).to_string()
        );

        paginated_query
            .load_and_count_pages::<SubscriptionForDisplay>(conn)
            .await
            .attach_printable("Error while fetching subscriptions")
            .into_db_result()
    }

    pub async fn update_subscription_mrr_delta(
        conn: &mut PgConn,
        subscription_id: uuid::Uuid,
        mrr_cents_delta: i64,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl::*;
        use diesel_async::RunQueryDsl;

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
        use diesel_async::RunQueryDsl;

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
