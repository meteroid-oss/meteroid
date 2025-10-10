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
    let raw_query = r"SELECT * from pgmq.send_batch($1, $2, $3) as msg_id";

    let (messages, headers): (Vec<_>, Vec<_>) =
        batch.iter().map(|row| (&row.message, &row.headers)).unzip();

    diesel::sql_query(raw_query)
        .bind::<sql_types::Text, _>(queue)
        .bind::<sql_types::Array<sql_types::Nullable<sql_types::Jsonb>>, _>(messages)
        .bind::<sql_types::Array<sql_types::Nullable<sql_types::Jsonb>>, _>(headers)
        .execute(conn)
        .await
        .map(drop)
        .attach("Error while sending batch of messages to pgmq")
        .into_db_result()
}

pub async fn read(
    conn: &mut PgConn,
    queue: &str,
    limit: MessageReadQty,
    vt: MessageReadVtSec,
) -> DbResult<Vec<PgmqMessageRow>> {
    let raw_query = r"SELECT * from pgmq.read($1, $2, $3)";

    diesel::sql_query(raw_query)
        .bind::<sql_types::Text, _>(queue)
        .bind::<sql_types::Int2, _>(limit)
        .bind::<sql_types::Int2, _>(vt)
        .get_results::<PgmqMessageRow>(conn)
        .await
        .attach("Error while reading messages from pgmq")
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
    let raw_query = format!(r"SELECT * from pgmq.a_{queue} where msg_id = any($1)");

    diesel::sql_query(raw_query)
        .bind::<sql_types::Array<sql_types::BigInt>, _>(ids)
        .get_results::<PgmqMessageRow>(conn)
        .await
        .attach("Error while listing archived messages from pgmq")
        .into_db_result()
}
