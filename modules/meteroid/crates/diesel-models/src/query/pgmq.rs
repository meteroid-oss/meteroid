use crate::errors::IntoDbResult;
use crate::pgmq::{MessageId, MessageReadQty, MessageReadVtSec, PgmqRow, PgmqRowNew};
use crate::{DbResult, PgConn};
use diesel::sql_types;
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

pub async fn send_batch(conn: &mut PgConn, queue: &str, batch: &[PgmqRowNew]) -> DbResult<()> {
    let raw_query = r#"SELECT * from pgmq.send_batch($1, $2, $3) as msg_id"#;

    let (messages, headers): (Vec<_>, Vec<_>) =
        batch.iter().map(|row| (&row.message, &row.headers)).unzip();

    diesel::sql_query(raw_query)
        .bind::<sql_types::Text, _>(queue)
        .bind::<sql_types::Array<sql_types::Nullable<sql_types::Jsonb>>, _>(messages)
        .bind::<sql_types::Array<sql_types::Nullable<sql_types::Jsonb>>, _>(headers)
        .execute(conn)
        .await
        .map(drop)
        .attach_printable("Error while sending batch of messages to pgmq")
        .into_db_result()
}

pub async fn read(
    conn: &mut PgConn,
    queue: &str,
    limit: MessageReadQty,
    vt: MessageReadVtSec,
) -> DbResult<Vec<PgmqRow>> {
    let raw_query = r#"SELECT * from pgmq.read($1, $2, $3)"#;

    diesel::sql_query(raw_query)
        .bind::<sql_types::Text, _>(queue)
        .bind::<sql_types::Int2, _>(limit)
        .bind::<sql_types::Int2, _>(vt)
        .get_results::<PgmqRow>(conn)
        .await
        .attach_printable("Error while reading messages from pgmq")
        .into_db_result()
}

pub async fn archive(conn: &mut PgConn, queue: &str, ids: &[MessageId]) -> DbResult<()> {
    let raw_query = r#"SELECT * from pgmq.archive($1, $2) as msg_id"#;

    diesel::sql_query(raw_query)
        .bind::<sql_types::Text, _>(queue)
        .bind::<sql_types::Array<sql_types::BigInt>, _>(ids)
        .execute(conn)
        .await
        .map(drop)
        .attach_printable("Error while archiving batch of messages to pgmq")
        .into_db_result()
}
