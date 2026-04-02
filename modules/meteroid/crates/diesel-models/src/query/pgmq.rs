use crate::errors::IntoDbResult;
use crate::pgmq::{PgmqMessageRow, PgmqMessageRowNew};
use crate::{DbResult, PgConn};
use common_domain::pgmq::{MessageId, MessageReadQty, MessageReadVtSec};
use diesel::sql_types;
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

pub async fn send_batch(
    conn: &mut PgConn,
    queue: &str,
    batch: &[PgmqMessageRowNew],
) -> DbResult<()> {
    send_batch_returning_ids(conn, queue, batch)
        .await
        .map(drop)
}

pub async fn send_batch_returning_ids(
    conn: &mut PgConn,
    queue: &str,
    batch: &[PgmqMessageRowNew],
) -> DbResult<Vec<i64>> {
    #[derive(diesel::QueryableByName)]
    struct SendResult {
        #[diesel(sql_type = sql_types::BigInt)]
        msg_id: i64,
    }

    let raw_query = r"SELECT * from pgmq.send_batch($1, $2, $3) as msg_id";

    let (messages, headers): (Vec<_>, Vec<_>) =
        batch.iter().map(|row| (&row.message, &row.headers)).unzip();

    diesel::sql_query(raw_query)
        .bind::<sql_types::Text, _>(queue)
        .bind::<sql_types::Array<sql_types::Nullable<sql_types::Jsonb>>, _>(messages)
        .bind::<sql_types::Array<sql_types::Nullable<sql_types::Jsonb>>, _>(headers)
        .get_results::<SendResult>(conn)
        .await
        .map(|rows| rows.into_iter().map(|r| r.msg_id).collect())
        .attach("Error while sending batch of messages to pgmq")
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

pub async fn list_archived(
    conn: &mut PgConn,
    queue: &str,
    ids: &[MessageId],
) -> DbResult<Vec<PgmqMessageRow>> {
    let raw_query = format!(
        r"SELECT msg_id, read_ct, message, headers, enqueued_at FROM pgmq.a_{queue} WHERE msg_id = any($1)"
    );

    diesel::sql_query(raw_query)
        .bind::<sql_types::Array<sql_types::BigInt>, _>(ids)
        .get_results::<PgmqMessageRow>(conn)
        .await
        .attach("Error while listing archived messages from pgmq")
        .into_db_result()
}
