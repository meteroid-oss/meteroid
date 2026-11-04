use crate::errors::IntoDbResult;
use crate::pgmq::{PgmqMessageRow, PgmqMessageRowNew};
use crate::{DbResult, PgConn};
use common_domain::pgmq::{MessageId, MessageReadQty, MessageReadVtSec};
use diesel::QueryableByName;
use diesel::sql_types;
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

pub async fn send_batch(
    conn: &mut PgConn,
    queue: &str,
    batch: &[PgmqMessageRowNew],
) -> DbResult<()> {
    send_batch_delayed(conn, queue, batch, 0).await
}

/// Enqueues messages that only become visible after `delay_seconds`. Used for
/// scheduled retries (e.g. re-checking VIES after an outage).
pub async fn send_batch_delayed(
    conn: &mut PgConn,
    queue: &str,
    batch: &[PgmqMessageRowNew],
    delay_seconds: i32,
) -> DbResult<()> {
    let raw_query = r"SELECT * from pgmq.send_batch($1, $2, $3, $4) as msg_id";

    let (messages, headers): (Vec<_>, Vec<_>) =
        batch.iter().map(|row| (&row.message, &row.headers)).unzip();

    diesel::sql_query(raw_query)
        .bind::<sql_types::Text, _>(queue)
        .bind::<sql_types::Array<sql_types::Nullable<sql_types::Jsonb>>, _>(messages)
        .bind::<sql_types::Array<sql_types::Nullable<sql_types::Jsonb>>, _>(headers)
        .bind::<sql_types::Integer, _>(delay_seconds)
        .execute(conn)
        .await
        .map(drop)
        .attach("Error while sending batch of messages to pgmq")
        .into_db_result()
}

#[derive(QueryableByName)]
struct TopicDeliveryCount {
    #[diesel(sql_type = sql_types::BigInt)]
    count: i64,
}

/// Publishes a batch of messages sharing one routing key; pgmq copies each into
/// every queue whose bound pattern matches, in this transaction. Returns the number
/// of (queue, message) deliveries — `0` means no binding matched the key.
pub async fn send_batch_topic(
    conn: &mut PgConn,
    routing_key: &str,
    batch: &[PgmqMessageRowNew],
    delay_seconds: i32,
) -> DbResult<i64> {
    let raw_query = r"SELECT count(*) AS count FROM pgmq.send_batch_topic($1, $2, $3, $4)";

    let (messages, headers): (Vec<_>, Vec<_>) =
        batch.iter().map(|row| (&row.message, &row.headers)).unzip();

    diesel::sql_query(raw_query)
        .bind::<sql_types::Text, _>(routing_key)
        .bind::<sql_types::Array<sql_types::Nullable<sql_types::Jsonb>>, _>(messages)
        .bind::<sql_types::Array<sql_types::Nullable<sql_types::Jsonb>>, _>(headers)
        .bind::<sql_types::Integer, _>(delay_seconds)
        .get_result::<TopicDeliveryCount>(conn)
        .await
        .map(|r| r.count)
        .attach(format!(
            "Error while publishing batch to topic {routing_key}"
        ))
        .into_db_result()
}

pub async fn read(
    conn: &mut PgConn,
    queue: &str,
    limit: MessageReadQty,
    vt: MessageReadVtSec,
) -> DbResult<Vec<PgmqMessageRow>> {
    let raw_query =
        r"SELECT msg_id, read_ct, message, headers, enqueued_at FROM pgmq.read($1, $2, $3)";

    diesel::sql_query(raw_query)
        .bind::<sql_types::Text, _>(queue)
        .bind::<sql_types::Int2, _>(vt)
        .bind::<sql_types::Int2, _>(limit)
        .get_results::<PgmqMessageRow>(conn)
        .await
        .attach(format!("Error while reading messages from pgmq {queue}"))
        .into_db_result()
}

/// Pushes each message's visibility to `now() + vt` seconds, per message.
pub async fn set_vt(
    conn: &mut PgConn,
    queue: &str,
    ids: &[MessageId],
    vts: &[i32],
) -> DbResult<()> {
    let raw_query = r"SELECT 1 FROM unnest($2::bigint[], $3::int[]) AS t(msg_id, vt), LATERAL pgmq.set_vt($1, t.msg_id, t.vt)";

    diesel::sql_query(raw_query)
        .bind::<sql_types::Text, _>(queue)
        .bind::<sql_types::Array<sql_types::BigInt>, _>(ids)
        .bind::<sql_types::Array<sql_types::Integer>, _>(vts)
        .execute(conn)
        .await
        .map(drop)
        .attach(format!(
            "Error while setting visibility timeout on pgmq {queue}"
        ))
        .into_db_result()
}

pub async fn archive(conn: &mut PgConn, queue: &str, ids: &[MessageId]) -> DbResult<()> {
    let raw_query = r"SELECT * from pgmq.archive($1, $2) as msg_id";

    diesel::sql_query(raw_query)
        .bind::<sql_types::Text, _>(queue)
        .bind::<sql_types::Array<sql_types::BigInt>, _>(ids)
        .execute(conn)
        .await
        .map(drop)
        .attach("Error while archiving batch of messages to pgmq")
        .into_db_result()
}

pub async fn delete(conn: &mut PgConn, queue: &str, ids: &[MessageId]) -> DbResult<()> {
    let raw_query = r"SELECT * from pgmq.delete($1, $2) as msg_id";

    diesel::sql_query(raw_query)
        .bind::<sql_types::Text, _>(queue)
        .bind::<sql_types::Array<sql_types::BigInt>, _>(ids)
        .execute(conn)
        .await
        .map(drop)
        .attach("Error while deleting batch of messages to pgmq")
        .into_db_result()
}
