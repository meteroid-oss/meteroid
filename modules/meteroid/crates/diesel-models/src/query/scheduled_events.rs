//! Repository for scheduled events

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

use crate::enums::{ScheduledEventStatus, ScheduledEventTypeEnum};
use crate::errors::IntoDbResult;
use crate::scheduled_events::{ScheduledEventRow, ScheduledEventRowNew};
use crate::subscription_facts::SubscriptionEndingRow;
use crate::{DbResult, PgConn};
use common_domain::ids::{ScheduledEventId, SubscriptionId, TenantId};

impl ScheduledEventRow {
    /// Get event by ID
    pub async fn get_by_id(
        conn: &mut PgConn,
        event_id_param: ScheduledEventId,
        tenant_id_param: &TenantId,
    ) -> DbResult<ScheduledEventRow> {
        use crate::schema::scheduled_event::dsl::{id, scheduled_event, tenant_id};

        let query = scheduled_event
            .filter(id.eq(event_id_param))
            .filter(tenant_id.eq(tenant_id_param));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while fetching scheduled event by ID")
            .into_db_result()
    }

    /// Get all pending events for a subscription (internal use)
    pub async fn get_pending_events_for_subscription(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
        tenant_id_param: &TenantId,
    ) -> DbResult<Vec<ScheduledEventRow>> {
        use crate::schema::scheduled_event::dsl::{
            scheduled_event, scheduled_time, subscription_id, tenant_id,
        };

        let query = scheduled_event
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(still_pending())
            .order_by(scheduled_time.asc());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while fetching pending scheduled events")
            .into_db_result()
    }

    /// Get pending user-visible events for a subscription.
    /// Filters to lifecycle events (plan change, cancel, pause, end trial) only —
    /// excludes internal billing events (finalize invoice, retry payment).
    pub async fn get_pending_user_visible_events_for_subscription(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
        tenant_id_param: &TenantId,
    ) -> DbResult<Vec<ScheduledEventRow>> {
        use crate::schema::scheduled_event::dsl::{
            event_type, scheduled_event, scheduled_time, subscription_id, tenant_id,
        };

        let visible_types = vec![
            ScheduledEventTypeEnum::ApplyPlanChange,
            ScheduledEventTypeEnum::ApplyAmendment,
            ScheduledEventTypeEnum::CancelSubscription,
            ScheduledEventTypeEnum::PauseSubscription,
            ScheduledEventTypeEnum::EndTrial,
        ];

        let query = scheduled_event
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(still_pending())
            .filter(event_type.eq_any(visible_types))
            .order_by(scheduled_time.asc());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while fetching pending user-visible scheduled events")
            .into_db_result()
    }

    /// Get the next pending cancellation date for each of the given subscriptions.
    /// Used to surface "cancels on" instead of "renews on" in list/overview views.
    pub async fn list_pending_cancellations_for_subscriptions(
        conn: &mut PgConn,
        subscription_ids: &[SubscriptionId],
        tenant_id_param: &TenantId,
    ) -> DbResult<Vec<(SubscriptionId, NaiveDateTime)>> {
        use crate::schema::scheduled_event::dsl::{
            event_type, scheduled_event, scheduled_time, status, subscription_id, tenant_id,
        };

        let query = scheduled_event
            .filter(subscription_id.eq_any(subscription_ids))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(status.eq(ScheduledEventStatus::Pending))
            .filter(event_type.eq(ScheduledEventTypeEnum::CancelSubscription))
            .order_by(scheduled_time.asc())
            .select((subscription_id, scheduled_time));

        query
            .get_results(conn)
            .await
            .attach("Error while fetching pending cancellations")
            .into_db_result()
    }

    /// The next attempt of the earliest change of a subscription dated before `before` (cancel,
    /// pause, plan change, amendment) still pending or being applied; `Some(None)` when it is due.
    pub async fn next_unapplied_change_before(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_id_param: SubscriptionId,
        before: NaiveDateTime,
    ) -> DbResult<Option<Option<NaiveDateTime>>> {
        use crate::schema::scheduled_event::dsl::{
            event_type, next_attempt_at, scheduled_event, scheduled_time, status, subscription_id,
            tenant_id,
        };

        scheduled_event
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(status.eq_any([
                ScheduledEventStatus::Pending,
                ScheduledEventStatus::Processing,
            ]))
            .filter(event_type.eq_any([
                ScheduledEventTypeEnum::CancelSubscription,
                ScheduledEventTypeEnum::PauseSubscription,
                ScheduledEventTypeEnum::ApplyPlanChange,
                ScheduledEventTypeEnum::ApplyAmendment,
            ]))
            .filter(scheduled_time.lt(before))
            .order_by(scheduled_time.asc())
            .select(next_attempt_at)
            .first::<Option<NaiveDateTime>>(conn)
            .await
            .optional()
            .attach("Error while looking for an unapplied change of a subscription")
            .into_db_result()
    }

    /// Get pending events for a subscription
    pub async fn get_pending_events_for_subscription_between_dates(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
        tenant_id_param: &TenantId,
        start_date: NaiveDateTime,
        end_date: NaiveDateTime,
    ) -> DbResult<Vec<ScheduledEventRow>> {
        use crate::schema::scheduled_event::dsl::{
            scheduled_event, scheduled_time, status, subscription_id, tenant_id,
        };

        let query = scheduled_event
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(scheduled_time.ge(start_date))
            .filter(scheduled_time.lt(end_date))
            .filter(status.eq(ScheduledEventStatus::Pending))
            .order_by(scheduled_time.asc());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while fetching pending scheduled events")
            .into_db_result()
    }

    /// Get next event by type for a subscription
    pub async fn get_event_by_types_and_date_for_update(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
        tenant_id_param: &TenantId,
        event_type_param: Vec<ScheduledEventTypeEnum>,
        date: NaiveDate,
    ) -> DbResult<Option<ScheduledEventRow>> {
        use crate::schema::scheduled_event::dsl::{
            created_at, event_type, processed_at, scheduled_event, scheduled_time, status,
            subscription_id, tenant_id,
        };

        // Use range query instead of DATE() function to allow index usage
        let day_start = date.and_time(NaiveTime::MIN);
        let day_end = date.succ_opt().unwrap_or(date).and_time(NaiveTime::MIN);

        let query = scheduled_event
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(event_type.eq_any(event_type_param))
            .filter(status.eq(ScheduledEventStatus::Pending))
            .filter(processed_at.is_null())
            .filter(scheduled_time.ge(day_start))
            .filter(scheduled_time.lt(day_end))
            .order_by((
                // priority.desc(),
                created_at.desc(),
            ))
            .for_update()
            .skip_locked();

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while fetching scheduled events by type")
            .into_db_result()
    }

    /// Find and claim due events in a single atomic operation.
    /// Raw SQL is required because:
    /// 1. FOR UPDATE OF <table> syntax - Diesel's for_update() locks ALL joined tables
    /// 2. UPDATE ... RETURNING with subquery - not expressible in Diesel's query builder
    ///
    /// `customer`: only that customer's (its billing controller's run).
    pub async fn find_and_claim_due_events(
        conn: &mut PgConn,
        limit: i64,
        customer: Option<(TenantId, common_domain::ids::CustomerId)>,
    ) -> DbResult<Vec<ScheduledEventRow>> {
        use diesel::sql_types;

        let raw_sql = r#"
            UPDATE scheduled_event
            SET status = 'PROCESSING', updated_at = NOW()
            WHERE id IN (
                SELECT se.id
                FROM scheduled_event se
                INNER JOIN subscription s ON se.subscription_id = s.id
                -- Only ACTIVE tenants are processed; paused/archived are skipped.
                -- FOR UPDATE OF se (below) keeps this join out of the lock scope.
                INNER JOIN tenant t ON t.id = se.tenant_id AND t.status = 'ACTIVE'
                -- A sandbox still being seeded advances its subscriptions itself.
                LEFT JOIN sandbox_seed sd ON sd.tenant_id = se.tenant_id AND sd.status = 'IN_PROGRESS'
                WHERE se.status = 'PENDING'
                  AND sd.tenant_id IS NULL
                  AND ($3::uuid IS NULL OR (se.tenant_id = $3 AND s.customer_id = $4))
                  AND se.scheduled_time <= $1
                  AND (se.next_attempt_at IS NULL OR se.next_attempt_at <= $1)
                  AND (
                      -- Non-lifecycle events: always process
                      se.event_type IN ('FINALIZE_INVOICE', 'RETRY_PAYMENT')
                      -- Subscription already terminated: no lifecycle to wait for
                      OR s.current_period_end IS NULL
                      -- Event scheduled before period boundary: safe to process now
                      OR s.current_period_end > se.scheduled_time::date
                  )
                ORDER BY se.scheduled_time ASC, se.priority DESC
                LIMIT $2
                FOR UPDATE OF se SKIP LOCKED
            )
            RETURNING id, subscription_id, tenant_id, event_type, scheduled_time,
                      priority, event_data, created_at, updated_at, status,
                      retries, last_retry_at, error, processed_at, source,
                      created_by_customer, actor_type, actor_uuid, actor_alias,
                      next_attempt_at, written_facts
        "#;

        let now = common_clock::now().naive_utc();

        let query = diesel::sql_query(raw_sql)
            .bind::<sql_types::Timestamp, _>(now)
            .bind::<sql_types::BigInt, _>(limit)
            .bind::<sql_types::Nullable<sql_types::Uuid>, _>(customer.map(|(tenant, _)| tenant))
            .bind::<sql_types::Nullable<sql_types::Uuid>, _>(customer.map(|(_, id)| id));

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while claiming due events")
            .into_db_result()
    }

    /// Mark event as processing
    pub async fn mark_as_processing(
        conn: &mut PgConn,
        event_id_param: &[ScheduledEventId],
    ) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{id, scheduled_event, status, updated_at};

        let query = diesel::update(scheduled_event)
            .filter(id.eq_any(event_id_param))
            .set((
                status.eq(ScheduledEventStatus::Processing),
                updated_at.eq(common_clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while marking events as processing")
            .map(|_| ())
            .into_db_result()
    }

    /// Mark event as completed
    pub async fn mark_as_completed(
        conn: &mut PgConn,
        event_id_param: &[ScheduledEventId],
    ) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{
            id, processed_at, scheduled_event, status, updated_at,
        };

        // A change written as facts keeps its MIGRATED record (rollback, the API's pending list).
        let query = diesel::update(scheduled_event)
            .filter(id.eq_any(event_id_param))
            .filter(status.ne(ScheduledEventStatus::Migrated))
            .set((
                status.eq(ScheduledEventStatus::Completed),
                processed_at.eq(Some(common_clock::now().naive_utc())),
                updated_at.eq(common_clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while marking event as completed")
            .map(|_| ())
            .into_db_result()
    }

    /// Mark event as failed
    pub async fn mark_as_failed(
        conn: &mut PgConn,
        event_id_param: &ScheduledEventId,
        error_message: &str,
    ) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{error, id, scheduled_event, status, updated_at};

        let query = diesel::update(scheduled_event)
            .filter(id.eq(event_id_param))
            .set((
                status.eq(ScheduledEventStatus::Failed),
                error.eq(Some(error_message)),
                updated_at.eq(common_clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while marking event as failed")
            .map(|_| ())
            .into_db_result()
    }

    /// Records why an event did nothing, its status kept.
    pub async fn record_error(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        event_id_param: &ScheduledEventId,
        error_message: &str,
    ) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{
            error, id, scheduled_event, tenant_id, updated_at,
        };

        let query = diesel::update(scheduled_event)
            .filter(id.eq(event_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .set((
                error.eq(Some(error_message)),
                updated_at.eq(common_clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while recording an event's error")
            .map(|_| ())
            .into_db_result()
    }

    /// Marks an event written as facts, with the rows it wrote; returns whether it was still
    /// pending, or claimed (a straggler an old replica left `PROCESSING` is migrated, not applied).
    pub async fn mark_as_migrated(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        event_id_param: &ScheduledEventId,
        written: Option<serde_json::Value>,
    ) -> DbResult<bool> {
        use crate::schema::scheduled_event::dsl::{
            id, processed_at, scheduled_event, status, tenant_id, updated_at, written_facts,
        };

        let now = common_clock::now().naive_utc();
        let query = diesel::update(scheduled_event)
            .filter(id.eq(event_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(status.eq_any([
                ScheduledEventStatus::Pending,
                ScheduledEventStatus::Processing,
            ]))
            .set((
                status.eq(ScheduledEventStatus::Migrated),
                processed_at.eq(Some(now)),
                updated_at.eq(now),
                written_facts.eq(written),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while marking event as migrated")
            .map(|updated| updated > 0)
            .into_db_result()
    }

    /// Cleanup stale PROCESSING events. If an event has been in PROCESSING state
    /// for longer than [`CLAIM_TIMEOUT_MINUTES`], reset it to PENDING for retry (worker likely
    /// crashed).
    pub async fn retry_timeout_events(conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{
            error, last_retry_at, next_attempt_at, retries, scheduled_event, status, updated_at,
        };

        let now = common_clock::now().naive_utc();

        let query = diesel::update(scheduled_event)
            .filter(
                status
                    .eq(ScheduledEventStatus::Processing)
                    // Use updated_at (when marked PROCESSING), not scheduled_time
                    .and(updated_at.le(stale_claims_before(now))),
            )
            .set((
                status.eq(ScheduledEventStatus::Pending),
                next_attempt_at.eq(Some(now)),
                retries.eq(retries + 1),
                last_retry_at.eq(Some(now)),
                error.eq(Some("timeout".to_string())),
                updated_at.eq(now),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while marking event as failed")
            .map(|_| ())
            .into_db_result()
    }

    /// Retry an event at `retry_time`, counting the attempt; its effective date stays.
    pub async fn retry_event(
        conn: &mut PgConn,
        event_id_param: &ScheduledEventId,
        retry_time: NaiveDateTime,
        error_message: &str,
    ) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{
            error, id, last_retry_at, next_attempt_at, retries, scheduled_event, status, updated_at,
        };

        let query = diesel::update(scheduled_event)
            .filter(id.eq(event_id_param))
            .set((
                status.eq(ScheduledEventStatus::Pending),
                next_attempt_at.eq(Some(retry_time)),
                retries.eq(retries + 1),
                last_retry_at.eq(Some(common_clock::now().naive_utc())),
                error.eq(Some(error_message)),
                updated_at.eq(common_clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while scheduling event retry")
            .map(|_| ())
            .into_db_result()
    }

    /// Tries an event again at `retry_time` without counting the attempt: what it waits on is
    /// not its own failure.
    pub async fn defer_event(
        conn: &mut PgConn,
        event_id_param: &ScheduledEventId,
        retry_time: NaiveDateTime,
        error_message: &str,
    ) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{
            error, id, next_attempt_at, scheduled_event, status, updated_at,
        };

        diesel::update(scheduled_event)
            .filter(id.eq(event_id_param))
            .set((
                status.eq(ScheduledEventStatus::Pending),
                next_attempt_at.eq(Some(retry_time)),
                error.eq(Some(error_message)),
                updated_at.eq(common_clock::now().naive_utc()),
            ))
            .execute(conn)
            .await
            .attach("Error while deferring a scheduled event")
            .map(|_| ())
            .into_db_result()
    }

    /// Cancels the event while it is still pending; returns whether it was.
    pub async fn cancel_event(
        conn: &mut PgConn,
        event_id_param: &ScheduledEventId,
        reason: &str,
    ) -> DbResult<bool> {
        use crate::schema::scheduled_event::dsl::{error, id, scheduled_event, status, updated_at};

        let query = diesel::update(scheduled_event)
            .filter(id.eq(event_id_param))
            .filter(status.eq(ScheduledEventStatus::Pending))
            .set((
                status.eq(ScheduledEventStatus::Canceled),
                error.eq(Some(reason.to_string())),
                updated_at.eq(common_clock::now().naive_utc()),
            ))
            .returning(crate::schema::scheduled_event::subscription_id);

        crate::debug_sql(&query);

        let cancelled: Vec<SubscriptionId> = query
            .get_results(conn)
            .await
            .attach("Error while canceling event")
            .into_db_result()?;
        for subscription in &cancelled {
            SubscriptionEndingRow::withdraw_cancellation(conn, *subscription).await?;
        }
        Ok(!cancelled.is_empty())
    }

    /// Cancel the pending plan changes and pauses a new plan change replaces. A scheduled
    /// cancellation stays: a plan change never undoes it (R-092).
    pub async fn cancel_pending_plan_changes(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
        tenant_id_param: &TenantId,
        reason: &str,
    ) -> DbResult<usize> {
        Self::cancel_pending_of(
            conn,
            subscription_id_param,
            tenant_id_param,
            reason,
            &[
                ScheduledEventTypeEnum::ApplyPlanChange,
                ScheduledEventTypeEnum::PauseSubscription,
            ],
        )
        .await
    }

    /// [`Self::cancel_pending_plan_changes`] for a plan change ending a paid trial: its trial end
    /// goes too. A scheduled cancellation stays (R-092).
    pub async fn cancel_pending_trial_end_and_plan_changes(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
        tenant_id_param: &TenantId,
        reason: &str,
    ) -> DbResult<usize> {
        Self::cancel_pending_of(
            conn,
            subscription_id_param,
            tenant_id_param,
            reason,
            &[
                ScheduledEventTypeEnum::ApplyPlanChange,
                ScheduledEventTypeEnum::PauseSubscription,
                ScheduledEventTypeEnum::EndTrial,
            ],
        )
        .await
    }

    async fn cancel_pending_of(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
        tenant_id_param: &TenantId,
        reason: &str,
        types: &[ScheduledEventTypeEnum],
    ) -> DbResult<usize> {
        use crate::schema::scheduled_event::dsl::{
            error, event_type, scheduled_event, status, subscription_id, tenant_id, updated_at,
        };

        let query = diesel::update(scheduled_event)
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(status.eq(ScheduledEventStatus::Pending))
            .filter(event_type.eq_any(types))
            .set((
                status.eq(ScheduledEventStatus::Canceled),
                error.eq(Some(reason.to_string())),
                updated_at.eq(common_clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while canceling pending plan changes")
            .into_db_result()
    }

    /// Cancel all pending user-initiated lifecycle events for a subscription (plan change, pause, cancel).
    /// EndTrial is excluded: it's an internal timer that must always fire to transition out of TrialActive.
    pub async fn cancel_pending_lifecycle_events(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
        tenant_id_param: &TenantId,
        reason: &str,
    ) -> DbResult<usize> {
        use crate::schema::scheduled_event::dsl::{
            error, event_type, scheduled_event, status, subscription_id, tenant_id, updated_at,
        };

        let lifecycle_types = vec![
            ScheduledEventTypeEnum::ApplyPlanChange,
            ScheduledEventTypeEnum::PauseSubscription,
            ScheduledEventTypeEnum::CancelSubscription,
        ];

        let query = diesel::update(scheduled_event)
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(status.eq(ScheduledEventStatus::Pending))
            .filter(event_type.eq_any(lifecycle_types))
            .set((
                status.eq(ScheduledEventStatus::Canceled),
                error.eq(Some(reason.to_string())),
                updated_at.eq(common_clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        let cancelled = query
            .execute(conn)
            .await
            .attach("Error while canceling pending lifecycle events")
            .into_db_result()?;
        if cancelled > 0 {
            SubscriptionEndingRow::withdraw_cancellation(conn, subscription_id_param).await?;
        }
        Ok(cancelled)
    }

    /// Cancel all pending subscription-level events (lifecycle + EndTrial).
    /// Used when terminating a subscription — the subscription is dead, so lifecycle and trial
    /// events are moot. Billing events (FinalizeInvoice, RetryPayment) are preserved.
    pub async fn cancel_pending_subscription_events(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
        tenant_id_param: &TenantId,
        reason: &str,
    ) -> DbResult<usize> {
        use crate::schema::scheduled_event::dsl::{
            error, event_type, scheduled_event, status, subscription_id, tenant_id, updated_at,
        };

        let subscription_types = vec![
            ScheduledEventTypeEnum::ApplyPlanChange,
            ScheduledEventTypeEnum::PauseSubscription,
            ScheduledEventTypeEnum::CancelSubscription,
            ScheduledEventTypeEnum::EndTrial,
        ];

        let query = diesel::update(scheduled_event)
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(status.eq(ScheduledEventStatus::Pending))
            .filter(event_type.eq_any(subscription_types))
            .set((
                status.eq(ScheduledEventStatus::Canceled),
                error.eq(Some(reason.to_string())),
                updated_at.eq(common_clock::now().naive_utc()),
            ));

        crate::debug_sql(&query);

        let cancelled = query
            .execute(conn)
            .await
            .attach("Error while canceling pending subscription events")
            .into_db_result()?;
        if cancelled > 0 {
            SubscriptionEndingRow::withdraw_cancellation(conn, subscription_id_param).await?;
        }
        Ok(cancelled)
    }
}

impl ScheduledEventRow {
    /// The changes written as facts the state has not applied yet, of `types`, earliest first.
    /// Read under the customer's lock, as every write of them.
    pub async fn list_migrated_pending(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_id_param: SubscriptionId,
        types: &[ScheduledEventTypeEnum],
    ) -> DbResult<Vec<ScheduledEventRow>> {
        use crate::schema::scheduled_event::dsl::{
            event_type, scheduled_event, scheduled_time, status, subscription_id, tenant_id,
        };

        let query = scheduled_event
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(status.eq(ScheduledEventStatus::Migrated))
            .filter(not_applied())
            .filter(event_type.eq_any(types))
            .order_by(scheduled_time.asc());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing the changes written as facts still to apply")
            .into_db_result()
    }

    /// Every plan change and amendment still pending, or claimed longer ago than the claim
    /// timeout, of the tenants the billing worker runs: what the deploy-time migration writes as
    /// facts. A fresher claim is an old replica's, still applying it.
    pub async fn list_pending_changes(
        conn: &mut PgConn,
    ) -> DbResult<Vec<(TenantId, ScheduledEventId)>> {
        use crate::schema::scheduled_event::dsl::{
            event_type, id, scheduled_event, scheduled_time, status, tenant_id, updated_at,
        };

        let claimed_before = stale_claims_before(common_clock::now().naive_utc());
        let query = scheduled_event
            .filter(
                status.eq(ScheduledEventStatus::Pending).or(status
                    .eq(ScheduledEventStatus::Processing)
                    .and(updated_at.le(claimed_before))),
            )
            .filter(event_type.eq_any([
                ScheduledEventTypeEnum::ApplyPlanChange,
                ScheduledEventTypeEnum::ApplyAmendment,
            ]))
            .filter(diesel::dsl::sql::<diesel::sql_types::Bool>(
                BILLED_TENANT_SQL,
            ))
            .order_by(scheduled_time.asc())
            .select((tenant_id, id));

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing the pending changes")
            .into_db_result()
    }

    /// Whether the subscription was ever paused: a pause has no fact (R29).
    pub async fn was_paused(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<bool> {
        use crate::schema::scheduled_event::dsl::{
            event_type, scheduled_event, status, subscription_id, tenant_id,
        };
        use diesel::dsl::exists;

        diesel::select(exists(
            scheduled_event
                .filter(subscription_id.eq(subscription_id_param))
                .filter(tenant_id.eq(tenant_id_param))
                .filter(event_type.eq(ScheduledEventTypeEnum::PauseSubscription))
                .filter(status.eq(ScheduledEventStatus::Completed)),
        ))
        .get_result(conn)
        .await
        .attach("Error while looking for a pause")
        .into_db_result()
    }

    /// The changes of `kind` written as facts that took effect by `at`, earliest first.
    pub async fn list_migrated_in_force(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_id_param: SubscriptionId,
        kind: ScheduledEventTypeEnum,
        at: NaiveDateTime,
    ) -> DbResult<Vec<ScheduledEventRow>> {
        use crate::schema::scheduled_event::dsl::{
            event_type, scheduled_event, scheduled_time, status, subscription_id, tenant_id,
        };

        let query = scheduled_event
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(status.eq(ScheduledEventStatus::Migrated))
            .filter(scheduled_time.le(at))
            .filter(event_type.eq(kind))
            .order_by(scheduled_time.asc());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing the changes written as facts in force")
            .into_db_result()
    }

    /// Cancels a change written as facts before the state applied it; returns whether it had not.
    pub async fn withdraw_migrated(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        event_id_param: &ScheduledEventId,
        reason: &str,
    ) -> DbResult<bool> {
        use crate::schema::scheduled_event::dsl::{
            error, id, scheduled_event, status, tenant_id, updated_at,
        };

        let now = common_clock::now().naive_utc();
        let query = diesel::update(scheduled_event)
            .filter(id.eq(event_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(status.eq(ScheduledEventStatus::Migrated))
            .filter(not_applied())
            .set((
                status.eq(ScheduledEventStatus::Canceled),
                error.eq(Some(reason.to_string())),
                updated_at.eq(now),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while withdrawing a change written as facts")
            .map(|updated| updated > 0)
            .into_db_result()
    }

    /// Moves the subscription's pending `EndTrial` to `to`; returns it and the time it had.
    pub async fn move_pending_end_trial(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        subscription_id_param: SubscriptionId,
        to: NaiveDateTime,
    ) -> DbResult<Option<(ScheduledEventId, NaiveDateTime)>> {
        use crate::schema::scheduled_event::dsl::{
            event_type, id, scheduled_event, scheduled_time, status, subscription_id, tenant_id,
            updated_at,
        };

        let pending: Option<(ScheduledEventId, NaiveDateTime)> = scheduled_event
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(event_type.eq(ScheduledEventTypeEnum::EndTrial))
            .filter(status.eq(ScheduledEventStatus::Pending))
            .select((id, scheduled_time))
            .first(conn)
            .await
            .optional()
            .attach("Error while reading the pending trial end")
            .into_db_result()?;
        let Some((event, was)) = pending else {
            return Ok(None);
        };
        diesel::update(scheduled_event)
            .filter(id.eq(event))
            .filter(tenant_id.eq(tenant_id_param))
            .set((
                scheduled_time.eq(to),
                updated_at.eq(common_clock::now().naive_utc()),
            ))
            .execute(conn)
            .await
            .attach("Error while moving the trial end")
            .into_db_result()?;
        Ok(Some((event, was)))
    }

    /// Puts a pending event back on `at` (a moved trial end, when its change is cancelled).
    pub async fn reschedule_pending(
        conn: &mut PgConn,
        tenant_id_param: &TenantId,
        event_id_param: ScheduledEventId,
        at: NaiveDateTime,
    ) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{
            id, scheduled_event, scheduled_time, status, tenant_id, updated_at,
        };

        diesel::update(scheduled_event)
            .filter(id.eq(event_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(status.eq(ScheduledEventStatus::Pending))
            .set((
                scheduled_time.eq(at),
                updated_at.eq(common_clock::now().naive_utc()),
            ))
            .execute(conn)
            .await
            .attach("Error while rescheduling an event")
            .map(|_| ())
            .into_db_result()
    }
}

/// How long a claim (`PROCESSING`) holds before it is taken for a crashed worker's.
pub const CLAIM_TIMEOUT_MINUTES: i64 = 5;

/// Claims made before this are stale at `now`.
pub fn stale_claims_before(now: NaiveDateTime) -> NaiveDateTime {
    now - chrono::Duration::minutes(CLAIM_TIMEOUT_MINUTES)
}

/// The tenants the billing worker runs (`find_and_claim_due_events`): active, and not a sandbox
/// still being seeded.
const BILLED_TENANT_SQL: &str = "EXISTS (SELECT 1 FROM tenant t \
     WHERE t.id = scheduled_event.tenant_id AND t.status = 'ACTIVE') \
     AND NOT EXISTS (SELECT 1 FROM sandbox_seed sd \
     WHERE sd.tenant_id = scheduled_event.tenant_id AND sd.status = 'IN_PROGRESS')";

/// A change written as facts the derived state has not applied yet: no Switch row names it.
const NOT_APPLIED_SQL: &str = "NOT EXISTS (SELECT 1 FROM subscription_event ev \
     WHERE ev.subscription_id = scheduled_event.subscription_id AND ev.event_type = 'SWITCH' \
       AND ev.details ->> 'event' = scheduled_event.id::text)";

/// Pending, or written as facts and not applied yet: a scheduled change shows as pending until
/// the state takes it, whether its event waits or its rows do.
fn still_pending() -> diesel::expression::SqlLiteral<diesel::sql_types::Bool> {
    diesel::dsl::sql::<diesel::sql_types::Bool>(&format!(
        "(scheduled_event.status = 'PENDING' \
          OR (scheduled_event.status = 'MIGRATED' AND {NOT_APPLIED_SQL}))"
    ))
}

fn not_applied() -> diesel::expression::SqlLiteral<diesel::sql_types::Bool> {
    diesel::dsl::sql::<diesel::sql_types::Bool>(NOT_APPLIED_SQL)
}

impl ScheduledEventRowNew {
    pub async fn insert_batch(
        conn: &mut PgConn,
        events: &[ScheduledEventRowNew],
    ) -> DbResult<Vec<ScheduledEventRow>> {
        use crate::schema::scheduled_event::dsl::scheduled_event;

        let query = diesel::insert_into(scheduled_event).values(
            events
                .iter()
                .map(|row| {
                    (
                        row,
                        crate::schema::scheduled_event::created_at
                            .eq(common_clock::now().naive_utc()),
                    )
                })
                .collect::<Vec<_>>(),
        );

        crate::debug_sql(&query);

        let inserted: Vec<ScheduledEventRow> = query
            .get_results(conn)
            .await
            .attach("Error while creating scheduled events")
            .into_db_result()?;
        crate::query::subscription_facts::record_scheduled_facts(conn, &inserted).await?;
        Ok(inserted)
    }
}
