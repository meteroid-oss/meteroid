use crate::dead_letter::{DeadLetterAlertStateRow, DeadLetterMessageRow, DeadLetterMessageRowNew};
use crate::enums::DeadLetterStatusEnum;
use crate::errors::IntoDbResult;
use crate::schema::dead_letter_alert_state;
use crate::schema::dead_letter_message;
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

pub async fn insert_batch(
    conn: &mut PgConn,
    entries: Vec<DeadLetterMessageRowNew>,
) -> DbResult<()> {
    diesel::insert_into(dead_letter_message::table)
        .values(&entries)
        .execute(conn)
        .await
        .map(drop)
        .attach("Failed to insert dead letter messages")
        .into_db_result()
}

pub async fn list(
    conn: &mut PgConn,
    queue_filter: Option<&str>,
    status_filter: Option<DeadLetterStatusEnum>,
    limit: i64,
    offset: i64,
) -> DbResult<Vec<DeadLetterMessageRow>> {
    let mut query = dead_letter_message::table
        .order(dead_letter_message::dead_lettered_at.desc())
        .into_boxed();

    if let Some(q) = queue_filter {
        query = query.filter(dead_letter_message::queue.eq(q));
    }
    if let Some(s) = status_filter {
        query = query.filter(dead_letter_message::status.eq(s));
    }

    query
        .limit(limit)
        .offset(offset)
        .get_results(conn)
        .await
        .attach("Failed to list dead letter messages")
        .into_db_result()
}

pub async fn count(
    conn: &mut PgConn,
    queue_filter: Option<&str>,
    status_filter: Option<DeadLetterStatusEnum>,
) -> DbResult<i64> {
    let mut query = dead_letter_message::table.into_boxed();

    if let Some(q) = queue_filter {
        query = query.filter(dead_letter_message::queue.eq(q));
    }
    if let Some(s) = status_filter {
        query = query.filter(dead_letter_message::status.eq(s));
    }

    query
        .count()
        .get_result(conn)
        .await
        .attach("Failed to count dead letter messages")
        .into_db_result()
}

pub async fn get_by_id(conn: &mut PgConn, id: Uuid) -> DbResult<DeadLetterMessageRow> {
    dead_letter_message::table
        .find(id)
        .first(conn)
        .await
        .attach("Failed to get dead letter message")
        .into_db_result()
}

pub async fn update_status(
    conn: &mut PgConn,
    id: Uuid,
    status: DeadLetterStatusEnum,
    resolved_by: Uuid,
    requeued_pgmq_msg_id: Option<i64>,
) -> DbResult<DeadLetterMessageRow> {
    diesel::update(dead_letter_message::table.find(id))
        .set((
            dead_letter_message::status.eq(status),
            dead_letter_message::resolved_at.eq(diesel::dsl::now),
            dead_letter_message::resolved_by.eq(Some(resolved_by)),
            dead_letter_message::requeued_pgmq_msg_id.eq(requeued_pgmq_msg_id),
        ))
        .get_result(conn)
        .await
        .attach("Failed to update dead letter message status")
        .into_db_result()
}

#[derive(Debug, Clone, QueryableByName)]
pub struct DeadLetterQueueStatsRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub queue: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub pending_count: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub requeued_count: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub discarded_count: i64,
}

pub async fn queue_stats(conn: &mut PgConn) -> DbResult<Vec<DeadLetterQueueStatsRow>> {
    diesel::sql_query(
        r"SELECT
            queue,
            COUNT(*) FILTER (WHERE status = 'PENDING') AS pending_count,
            COUNT(*) FILTER (WHERE status = 'REQUEUED') AS requeued_count,
            COUNT(*) FILTER (WHERE status = 'DISCARDED') AS discarded_count
        FROM dead_letter_message
        GROUP BY queue
        ORDER BY queue",
    )
    .get_results(conn)
    .await
    .attach("Failed to get dead letter queue stats")
    .into_db_result()
}

pub async fn get_alert_state(
    conn: &mut PgConn,
    queue: &str,
) -> DbResult<Option<DeadLetterAlertStateRow>> {
    dead_letter_alert_state::table
        .find(queue)
        .first(conn)
        .await
        .optional()
        .attach("Failed to get alert state")
        .into_db_result()
}

pub async fn upsert_alert_state(conn: &mut PgConn, queue: &str) -> DbResult<()> {
    diesel::insert_into(dead_letter_alert_state::table)
        .values((
            dead_letter_alert_state::queue.eq(queue),
            dead_letter_alert_state::last_alerted_at.eq(diesel::dsl::now),
        ))
        .on_conflict(dead_letter_alert_state::queue)
        .do_update()
        .set(dead_letter_alert_state::last_alerted_at.eq(diesel::dsl::now))
        .execute(conn)
        .await
        .map(drop)
        .attach("Failed to upsert alert state")
        .into_db_result()
}

pub async fn pending_since(
    conn: &mut PgConn,
    since: NaiveDateTime,
) -> DbResult<Vec<DeadLetterQueueStatsRow>> {
    diesel::sql_query(
        r"SELECT
            queue,
            COUNT(*) AS pending_count,
            0::bigint AS requeued_count,
            0::bigint AS discarded_count
        FROM dead_letter_message
        WHERE status = 'PENDING' AND dead_lettered_at > $1
        GROUP BY queue
        ORDER BY queue",
    )
    .bind::<diesel::sql_types::Timestamptz, _>(since)
    .get_results(conn)
    .await
    .attach("Failed to get pending dead letters since timestamp")
    .into_db_result()
}
