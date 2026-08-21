//! Session-scoped Postgres advisory locks for cross-instance leader election (e.g.
//! electing a single analytics-projection worker among scheduler replicas). Unlike the
//! xact-scoped lock used for invoice consolidation, a session lock lives for the
//! connection's lifetime — and because pooled connections persist across checkouts, it
//! must be released explicitly with [`advisory_unlock`], not merely by dropping the handle.

use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use diesel::{QueryableByName, sql_query, sql_types};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

#[derive(QueryableByName)]
struct BoolRow {
    #[diesel(sql_type = sql_types::Bool)]
    value: bool,
}

/// Try to acquire a session advisory lock without blocking. `true` = acquired (held until
/// [`advisory_unlock`] or the connection closes); `false` = another session holds it.
pub async fn try_advisory_lock(conn: &mut PgConn, key: i64) -> DbResult<bool> {
    sql_query("SELECT pg_try_advisory_lock($1) AS value")
        .bind::<sql_types::BigInt, _>(key)
        .get_result::<BoolRow>(conn)
        .await
        .attach("Error acquiring session advisory lock")
        .into_db_result()
        .map(|r| r.value)
}

/// Release a session advisory lock previously taken on this connection.
pub async fn advisory_unlock(conn: &mut PgConn, key: i64) -> DbResult<bool> {
    sql_query("SELECT pg_advisory_unlock($1) AS value")
        .bind::<sql_types::BigInt, _>(key)
        .get_result::<BoolRow>(conn)
        .await
        .attach("Error releasing session advisory lock")
        .into_db_result()
        .map(|r| r.value)
}

/// Cheap liveness probe on the lock connection. A dropped/closed connection releases the
/// session lock, so a failure here means leadership was lost and must be re-elected.
pub async fn connection_alive(conn: &mut PgConn) -> bool {
    sql_query("SELECT 1").execute(conn).await.is_ok()
}
