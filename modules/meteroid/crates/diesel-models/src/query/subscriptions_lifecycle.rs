use crate::errors::IntoDbResult;

use crate::subscriptions::{
    SubscriptionCycleErrorRowPatch, SubscriptionCycleRowPatch, SubscriptionRow,
};
use crate::{DbResult, PgConn};

use diesel::dsl::not;
use diesel::{ExpressionMethods, QueryDsl, debug_query};
use diesel_async::RunQueryDsl;

use crate::enums::SubscriptionStatusEnum;
use common_domain::ids::{SubscriptionId, TenantId};
use error_stack::ResultExt;
use uuid::Uuid;

impl SubscriptionRow {
    // pub async fn cancel_subscription(
    //     conn: &mut PgConn,
    //     params: CancelSubscriptionParams,
    // ) -> DbResult<()> {
    //     use crate::schema::subscription::dsl::*;
    //
    //     let query = diesel::update(subscription)
    //         .filter(id.eq(params.subscription_id))
    //         .filter(tenant_id.eq(params.tenant_id))
    //         .filter(canceled_at.is_null())
    //         .set((
    //             end_date.eq(params.billing_end_date),
    //             canceled_at.eq(params.canceled_at), // TODO drop
    //             cancellation_reason.eq(params.reason),
    //         ));
    //
    //     log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));
    //
    //     query
    //         .execute(conn)
    //         .await
    //         .attach_printable("Error while canceling subscription")
    //         .into_db_result()?;
    //
    //     Ok(())
    // }

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

    pub async fn get_due_subscription_for_update(
        conn: &mut PgConn,
        limit: i64,
    ) -> DbResult<Vec<SubscriptionRow>> {
        use crate::schema::subscription::dsl;

        let query = dsl::subscription
            .filter(dsl::current_period_end.le(chrono::Utc::now().naive_utc().date()))
            .filter(dsl::error_count.le(3))
            .filter(not(dsl::status.eq_any(vec![
                SubscriptionStatusEnum::Cancelled,
                SubscriptionStatusEnum::Completed,
                SubscriptionStatusEnum::Superseded,
                SubscriptionStatusEnum::Suspended,
            ])))
            .order_by(dsl::current_period_end.asc())
            .limit(limit)
            .for_update()
            .skip_locked();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let res = query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching due subscriptions")
            .into_db_result()?;

        Ok(res)
    }
}

impl SubscriptionCycleRowPatch {
    pub async fn patch(&self, conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::subscription::dsl;

        let query = diesel::update(dsl::subscription)
            .filter(dsl::id.eq(&self.id))
            .filter(dsl::tenant_id.eq(&self.tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while updating subscription cycles")
            .into_db_result()?;

        Ok(())
    }
}
impl SubscriptionCycleErrorRowPatch {
    pub async fn patch(&self, conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::subscription::dsl;

        let query = diesel::update(dsl::subscription)
            .filter(dsl::id.eq(&self.id))
            .filter(dsl::tenant_id.eq(&self.tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while updating subscription errors")
            .into_db_result()?;

        Ok(())
    }
}
