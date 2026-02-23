//! Repository for scheduled events

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Utc};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

use crate::enums::{ScheduledEventStatus, ScheduledEventTypeEnum};
use crate::errors::IntoDbResult;
use crate::scheduled_events::{ScheduledEventRow, ScheduledEventRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{SubscriptionId, TenantId};

impl ScheduledEventRow {
    /// Get event by ID
    pub async fn get_by_id(
        conn: &mut PgConn,
        event_id_param: Uuid,
        tenant_id_param: &TenantId,
    ) -> DbResult<ScheduledEventRow> {
        use crate::schema::scheduled_event::dsl::{id, scheduled_event, tenant_id};

        let query = scheduled_event
            .filter(id.eq(event_id_param))
            .filter(tenant_id.eq(tenant_id_param));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while fetching scheduled event by ID")
            .into_db_result()
    }

    /// Get pending events for a subscription
    pub async fn get_pending_events_for_subscription(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
        tenant_id_param: &TenantId,
    ) -> DbResult<Vec<ScheduledEventRow>> {
        use crate::schema::scheduled_event::dsl::{
            scheduled_event, scheduled_time, status, subscription_id, tenant_id,
        };

        let query = scheduled_event
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(status.eq(ScheduledEventStatus::Pending))
            .order_by(scheduled_time.asc());

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while fetching pending scheduled events")
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

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

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

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

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
    pub async fn find_and_claim_due_events(
        conn: &mut PgConn,
        limit: i64,
    ) -> DbResult<Vec<ScheduledEventRow>> {
        use diesel::sql_types;

        let raw_sql = r#"
            UPDATE scheduled_event
            SET status = 'PROCESSING', updated_at = NOW()
            WHERE id IN (
                SELECT se.id
                FROM scheduled_event se
                INNER JOIN subscription s ON se.subscription_id = s.id
                WHERE se.status = 'PENDING'
                  AND se.scheduled_time <= $1
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
                      retries, last_retry_at, error, processed_at, source
        "#;

        let now = Utc::now().naive_utc();

        let query = diesel::sql_query(raw_sql)
            .bind::<sql_types::Timestamp, _>(now)
            .bind::<sql_types::BigInt, _>(limit);

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while claiming due events")
            .into_db_result()
    }

    /// Mark event as processing
    pub async fn mark_as_processing(conn: &mut PgConn, event_id_param: &[Uuid]) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{id, scheduled_event, status, updated_at};

        let query = diesel::update(scheduled_event)
            .filter(id.eq_any(event_id_param))
            .set((
                status.eq(ScheduledEventStatus::Processing),
                updated_at.eq(Utc::now().naive_utc()),
            ));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while marking events as processing")
            .map(|_| ())
            .into_db_result()
    }

    /// Mark event as completed
    pub async fn mark_as_completed(conn: &mut PgConn, event_id_param: &[Uuid]) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{
            id, processed_at, scheduled_event, status, updated_at,
        };

        let query = diesel::update(scheduled_event)
            .filter(id.eq_any(event_id_param))
            .set((
                status.eq(ScheduledEventStatus::Completed),
                processed_at.eq(Some(Utc::now().naive_utc())),
                updated_at.eq(Utc::now().naive_utc()),
            ));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

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
        event_id_param: &Uuid,
        error_message: &str,
    ) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{error, id, scheduled_event, status, updated_at};

        let query = diesel::update(scheduled_event)
            .filter(id.eq(event_id_param))
            .set((
                status.eq(ScheduledEventStatus::Failed),
                error.eq(Some(error_message)),
                updated_at.eq(Utc::now().naive_utc()),
            ));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while marking event as failed")
            .map(|_| ())
            .into_db_result()
    }

    /// Cleanup stale PROCESSING events. If an event has been in PROCESSING state
    /// for more than 5 minutes, reset it to PENDING for retry (worker likely crashed).
    pub async fn retry_timeout_events(conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{
            error, last_retry_at, retries, scheduled_event, scheduled_time, status, updated_at,
        };

        let now = Utc::now().naive_utc();
        let timeout_threshold = now.checked_sub_signed(chrono::Duration::minutes(5));
        if timeout_threshold.is_none() {
            return Ok(());
        }

        let query = diesel::update(scheduled_event)
            .filter(
                status
                    .eq(ScheduledEventStatus::Processing)
                    // Use updated_at (when marked PROCESSING), not scheduled_time
                    .and(updated_at.le(timeout_threshold.unwrap())),
            )
            .set((
                status.eq(ScheduledEventStatus::Pending),
                scheduled_time.eq(now),
                retries.eq(retries + 1),
                last_retry_at.eq(Some(now)),
                error.eq(Some("timeout".to_string())),
                updated_at.eq(now),
            ));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while marking event as failed")
            .map(|_| ())
            .into_db_result()
    }

    /// Retry an event
    pub async fn retry_event(
        conn: &mut PgConn,
        event_id_param: &Uuid,
        retry_time: NaiveDateTime,
        error_message: &str,
    ) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{
            error, id, last_retry_at, retries, scheduled_event, scheduled_time, status, updated_at,
        };

        let query = diesel::update(scheduled_event)
            .filter(id.eq(event_id_param))
            .set((
                status.eq(ScheduledEventStatus::Pending),
                scheduled_time.eq(retry_time),
                retries.eq(retries + 1),
                last_retry_at.eq(Some(Utc::now().naive_utc())),
                error.eq(Some(error_message)),
                updated_at.eq(Utc::now().naive_utc()),
            ));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while scheduling event retry")
            .map(|_| ())
            .into_db_result()
    }

    /// Cancel event
    pub async fn cancel_event(
        conn: &mut PgConn,
        event_id_param: &Uuid,
        reason: &str,
    ) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{error, id, scheduled_event, status, updated_at};

        let query = diesel::update(scheduled_event)
            .filter(id.eq(event_id_param))
            .set((
                status.eq(ScheduledEventStatus::Canceled),
                error.eq(Some(reason.to_string())),
                updated_at.eq(Utc::now().naive_utc()),
            ));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while canceling event")
            .map(|_| ())
            .into_db_result()
    }

    /// Cancel all pending lifecycle events for a subscription (plan change, pause, etc.).
    /// Used when terminating a subscription to prevent stale events from executing.
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
            ScheduledEventTypeEnum::EndTrial,
        ];

        let query = diesel::update(scheduled_event)
            .filter(subscription_id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(status.eq(ScheduledEventStatus::Pending))
            .filter(event_type.eq_any(lifecycle_types))
            .set((
                status.eq(ScheduledEventStatus::Canceled),
                error.eq(Some(reason.to_string())),
                updated_at.eq(Utc::now().naive_utc()),
            ));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while canceling pending lifecycle events")
            .into_db_result()
    }
}

impl ScheduledEventRowNew {
    pub async fn insert_batch(
        conn: &mut PgConn,
        events: &[ScheduledEventRowNew],
    ) -> DbResult<Vec<ScheduledEventRow>> {
        use crate::schema::scheduled_event::dsl::scheduled_event;

        let query = diesel::insert_into(scheduled_event).values(events);

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while creating scheduled events")
            .into_db_result()
    }
}
