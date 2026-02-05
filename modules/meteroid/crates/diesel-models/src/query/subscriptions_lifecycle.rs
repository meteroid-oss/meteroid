use crate::errors::IntoDbResult;
use chrono::{Duration, NaiveDate, NaiveDateTime, Utc};

use crate::subscriptions::{
    SubscriptionCycleErrorRowPatch, SubscriptionCycleRowPatch, SubscriptionRow,
};
use crate::{DbResult, PgConn};

use diesel::{BoolExpressionMethods, ExpressionMethods, OptionalExtension, QueryDsl, debug_query};
use diesel_async::RunQueryDsl;

use crate::enums::{CycleActionEnum, SubscriptionStatusEnum};
use common_domain::ids::{SubscriptionId, TenantId};
use error_stack::ResultExt;
use uuid::Uuid;

/// How long before a claim expires and can be picked up by another worker.
const CLAIM_TIMEOUT_MINUTES: i64 = 5;

impl SubscriptionRow {
    #[allow(clippy::too_many_arguments)]
    pub async fn activate_subscription(
        conn: &mut PgConn,
        id: &SubscriptionId,
        tenant_id: &TenantId,
        current_period_start: NaiveDate,
        current_period_end: Option<NaiveDate>,
        next_cycle_action: Option<CycleActionEnum>,
        cycle_index: Option<i32>,
        status: SubscriptionStatusEnum,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl as s_dsl;

        let query = diesel::update(s_dsl::subscription)
            .filter(s_dsl::id.eq(id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(s_dsl::activated_at.is_null())
            .set((
                s_dsl::activated_at.eq(chrono::Utc::now().naive_utc()),
                s_dsl::current_period_start.eq(current_period_start),
                s_dsl::current_period_end.eq(current_period_end),
                s_dsl::next_cycle_action.eq(next_cycle_action),
                s_dsl::cycle_index.eq(cycle_index),
                s_dsl::status.eq(status),
                s_dsl::pending_checkout.eq(false),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while activating subscription")
            .into_db_result()?;

        Ok(())
    }

    /// Transitions a TrialExpired subscription to Active after payment.
    /// Unlike activate_subscription, this works on already-activated subscriptions.
    #[allow(clippy::too_many_arguments)]
    pub async fn transition_trial_expired_to_active(
        conn: &mut PgConn,
        id: &SubscriptionId,
        tenant_id: &TenantId,
        current_period_start: NaiveDate,
        current_period_end: Option<NaiveDate>,
        next_cycle_action: Option<CycleActionEnum>,
        cycle_index: Option<i32>,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl as s_dsl;

        let query = diesel::update(s_dsl::subscription)
            .filter(s_dsl::id.eq(id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(s_dsl::status.eq(SubscriptionStatusEnum::TrialExpired))
            .set((
                s_dsl::current_period_start.eq(current_period_start),
                s_dsl::current_period_end.eq(current_period_end),
                s_dsl::next_cycle_action.eq(next_cycle_action),
                s_dsl::cycle_index.eq(cycle_index),
                s_dsl::status.eq(SubscriptionStatusEnum::Active),
                s_dsl::pending_checkout.eq(false),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while transitioning subscription from TrialExpired to Active")
            .into_db_result()?;

        Ok(())
    }

    pub async fn update_subscription_mrr_delta(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        mrr_cents_delta: i64,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl::{id, mrr_cents, subscription};

        let query = diesel::update(subscription)
            .filter(id.eq(subscription_id))
            .set(mrr_cents.eq(mrr_cents + mrr_cents_delta));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while updating subscription MRR")
            .into_db_result()?;

        Ok(())
    }

    pub async fn lock_subscription_for_update(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl::{id, subscription};

        let query = subscription
            .for_update()
            .select(id)
            .filter(id.eq(subscription_id_param));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let _res: Uuid = query
            .get_result(conn)
            .await
            .attach("Error while locking subscription for update")
            .into_db_result()?;

        Ok(())
    }

    /// Claims subscription IDs for processing.
    ///
    /// Subscriptions are available if:
    /// - They are due (current_period_end <= today)
    /// - Not in a retry backoff period
    /// - Not in a terminal status
    /// - Not currently claimed (processing_started_at is null or expired)
    pub async fn claim_due_subscriptions(
        conn: &mut PgConn,
        limit: i64,
    ) -> DbResult<Vec<SubscriptionId>> {
        use crate::schema::subscription::dsl;

        let now = Utc::now().naive_utc();
        let claim_timeout = now - Duration::minutes(CLAIM_TIMEOUT_MINUTES);

        let active_statuses = SubscriptionStatusEnum::not_terminal();

        // Step 1: Find and lock available subscriptions (prevents concurrent claims)
        let select_query = dsl::subscription
            .select(dsl::id)
            .filter(dsl::current_period_end.le(now.date()))
            // Only process if next_retry is null (no error) or next_retry time has passed
            .filter(dsl::next_retry.is_null().or(dsl::next_retry.le(now)))
            // Not currently claimed (null or expired claim)
            .filter(
                dsl::processing_started_at
                    .is_null()
                    .or(dsl::processing_started_at.lt(claim_timeout)),
            )
            // Only active statuses eligible for processing
            .filter(dsl::status.eq_any(active_statuses.clone()))
            .order_by(dsl::current_period_end.asc())
            .limit(limit)
            .for_update()
            .skip_locked();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&select_query));

        let ids: Vec<SubscriptionId> = select_query
            .get_results(conn)
            .await
            .attach("Error while selecting due subscriptions")
            .into_db_result()?;

        if ids.is_empty() {
            return Ok(vec![]);
        }

        // Step 2: Claim them by setting processing_started_at
        let update_query = diesel::update(dsl::subscription)
            .filter(dsl::id.eq_any(&ids))
            .set(dsl::processing_started_at.eq(Some(now)));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&update_query));

        update_query
            .execute(conn)
            .await
            .attach("Error while claiming subscriptions")
            .into_db_result()?;

        Ok(ids)
    }

    /// Fetches and locks a subscription for processing, re-validating that it's still due.
    ///
    /// Returns None if the subscription no longer meets the criteria (e.g., was modified
    /// between claim and processing).
    pub async fn get_and_lock_for_processing(
        conn: &mut PgConn,
        id: SubscriptionId,
    ) -> DbResult<Option<SubscriptionRow>> {
        use crate::schema::subscription::dsl;

        let now = Utc::now().naive_utc();

        let active_statuses = SubscriptionStatusEnum::not_terminal();

        // Re-validate conditions and lock.
        let query = dsl::subscription
            .filter(dsl::id.eq(id))
            .filter(dsl::current_period_end.le(now.date()))
            .filter(dsl::next_retry.is_null().or(dsl::next_retry.le(now)))
            .filter(dsl::status.eq_any(active_statuses))
            .for_update()
            .skip_locked();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while locking subscription for processing")
            .into_db_result()
    }

    /// Releases a claim on a subscription (sets processing_started_at to null).
    /// Used when processing fails and we want to allow immediate retry by another worker.
    pub async fn release_claim(conn: &mut PgConn, id: SubscriptionId) -> DbResult<()> {
        use crate::schema::subscription::dsl;

        let query = diesel::update(dsl::subscription)
            .filter(dsl::id.eq(id))
            .set(dsl::processing_started_at.eq(None::<NaiveDateTime>));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while releasing subscription claim")
            .into_db_result()?;

        Ok(())
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
            .attach("Error while updating subscription cycles")
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
            .attach("Error while updating subscription errors")
            .into_db_result()?;

        Ok(())
    }
}
