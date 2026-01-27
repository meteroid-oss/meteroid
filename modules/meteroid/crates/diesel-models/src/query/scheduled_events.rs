//! Repository for scheduled events

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Utc};
use diesel::dsl::sql;
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

    /// Find due events ready for processing
    pub async fn find_due_events_for_update(
        conn: &mut PgConn,
        limit: i64,
    ) -> DbResult<Vec<ScheduledEventRow>> {
        use crate::schema::scheduled_event::dsl::{
            event_type, priority, scheduled_event, scheduled_time, status,
        };
        use crate::schema::subscription::dsl as sub_dsl;

        let query = scheduled_event
            .inner_join(sub_dsl::subscription)
            .filter(status.eq(ScheduledEventStatus::Pending))
            .filter(scheduled_time.le(Utc::now().naive_utc()))
            .filter(
                // We ignore terminal events related to subscriptions that have unprocessed lifecycle terms
                // either it's not a terminal event
                event_type
                    .eq_any(vec![
                        ScheduledEventTypeEnum::FinalizeInvoice,
                        ScheduledEventTypeEnum::RetryPayment,
                    ])
                    // or the subscription is terminated
                    .or(sub_dsl::current_period_end.is_null())
                    // the event is scheduled before the current period end
                    .or(sub_dsl::current_period_end
                        .gt(sql::<diesel::sql_types::Date>("DATE(scheduled_time)").nullable())),
            )
            .order_by((scheduled_time.asc(), priority.desc()))
            .limit(limit)
            .for_update()
            .skip_locked()
            .select(ScheduledEventRow::as_select());

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while fetching due events")
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

    /// cleanup. If the event is older than X minutes and in processing state, mark it for reprocessing
    pub async fn retry_timeout_events(conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::scheduled_event::dsl::{
            error, last_retry_at, retries, scheduled_event, scheduled_time, status, updated_at,
        };

        let now = Utc::now().naive_utc();
        let timeout_date = now.checked_sub_signed(chrono::Duration::minutes(30));
        if timeout_date.is_none() {
            return Ok(());
        }

        let query = diesel::update(scheduled_event)
            .filter(
                status
                    .eq(ScheduledEventStatus::Processing)
                    .and(scheduled_time.le(timeout_date.unwrap())),
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
                status.eq(ScheduledEventStatus::Cancelled),
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
