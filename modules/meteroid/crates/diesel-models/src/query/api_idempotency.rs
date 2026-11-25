use crate::api_idempotency::{ApiIdempotencyKeyRow, ApiIdempotencyKeyRowNew};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use chrono::{DateTime, Utc};
use common_domain::ids::TenantId;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper, sql_types};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

/// Whether this call is the one that claimed the key, or one of its replays.
pub enum ApiIdempotencyClaim {
    /// Nobody held this key before: the caller's write has not run yet.
    Fresh,
    /// Already claimed, by this call or an earlier one that committed.
    Existing(ApiIdempotencyKeyRow),
}

impl ApiIdempotencyKeyRowNew {
    /// Records the key unless it already exists, and returns the stored record either way.
    pub async fn claim(&self, conn: &mut PgConn) -> DbResult<ApiIdempotencyKeyRow> {
        use crate::schema::api_idempotency_key::dsl as k;
        diesel::insert_into(k::api_idempotency_key)
            .values(self)
            .on_conflict((k::tenant_id, k::operation, k::idempotency_key))
            .do_nothing()
            .execute(conn)
            .await
            .attach("Error while claiming an idempotency key")
            .into_db_result()?;
        k::api_idempotency_key
            .filter(k::tenant_id.eq(self.tenant_id))
            .filter(k::operation.eq(&self.operation))
            .filter(k::idempotency_key.eq(&self.idempotency_key))
            .select(ApiIdempotencyKeyRow::as_select())
            .first(conn)
            .await
            .attach("Error while reading an idempotency key")
            .into_db_result()
    }

    /// Claims the key on `conn`'s own transaction: a row returned here is committed only if the
    /// caller's transaction commits, so a claim that survives proves the write ran. Prefer this
    /// over `claim` when the write itself runs on the same connection and transaction.
    pub async fn claim_in_tx(&self, conn: &mut PgConn) -> DbResult<ApiIdempotencyClaim> {
        use crate::schema::api_idempotency_key::dsl as k;
        let inserted: Option<ApiIdempotencyKeyRow> = diesel::insert_into(k::api_idempotency_key)
            .values(self)
            .on_conflict((k::tenant_id, k::operation, k::idempotency_key))
            .do_nothing()
            .returning(ApiIdempotencyKeyRow::as_select())
            .get_result(conn)
            .await
            .optional()
            .attach("Error while claiming an idempotency key in its own transaction")
            .into_db_result()?;
        match inserted {
            Some(_) => Ok(ApiIdempotencyClaim::Fresh),
            None => {
                let existing = k::api_idempotency_key
                    .filter(k::tenant_id.eq(self.tenant_id))
                    .filter(k::operation.eq(&self.operation))
                    .filter(k::idempotency_key.eq(&self.idempotency_key))
                    .select(ApiIdempotencyKeyRow::as_select())
                    .first(conn)
                    .await
                    .attach("Error while reading an idempotency key")
                    .into_db_result()?;
                Ok(ApiIdempotencyClaim::Existing(existing))
            }
        }
    }
}

impl ApiIdempotencyKeyRow {
    /// Forgets a key whose request was rejected before it changed anything.
    pub async fn release(
        conn: &mut PgConn,
        tenant_id: TenantId,
        operation: &str,
        idempotency_key: &str,
        request_fingerprint: &[u8],
    ) -> DbResult<usize> {
        use crate::schema::api_idempotency_key::dsl as k;
        diesel::delete(
            k::api_idempotency_key
                .filter(k::tenant_id.eq(tenant_id))
                .filter(k::operation.eq(operation))
                .filter(k::idempotency_key.eq(idempotency_key))
                .filter(k::request_fingerprint.eq(request_fingerprint)),
        )
        .execute(conn)
        .await
        .attach("Error while releasing an idempotency key")
        .into_db_result()
    }
}

/// The response stored for an API v2 write, replayed to its retries: `None` while the first call
/// is still running (or when it was a v1 key, which keeps none).
pub struct ApiIdempotentResponse {
    pub status: i32,
    /// None on a response kept before its content type was.
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// A key's response columns as stored: status, content type, body.
type StoredColumns = (Option<i32>, Option<String>, Option<Vec<u8>>);

impl ApiIdempotencyKeyRow {
    /// The response kept for the key, if the call that claimed it finished.
    pub async fn stored_response(
        conn: &mut PgConn,
        tenant_id: TenantId,
        operation: &str,
        idempotency_key: &str,
    ) -> DbResult<Option<ApiIdempotentResponse>> {
        use crate::schema::api_idempotency_key::dsl as k;
        let row: Option<StoredColumns> = k::api_idempotency_key
            .filter(k::tenant_id.eq(tenant_id))
            .filter(k::operation.eq(operation))
            .filter(k::idempotency_key.eq(idempotency_key))
            .select((
                k::response_status,
                k::response_content_type,
                k::response_body,
            ))
            .first(conn)
            .await
            .optional()
            .attach("Error while reading an idempotent response")
            .into_db_result()?;
        Ok(match row {
            Some((Some(status), content_type, Some(body))) => Some(ApiIdempotentResponse {
                status,
                content_type,
                body,
            }),
            _ => None,
        })
    }

    /// Keeps the response of the call that claimed the key, for its replays.
    #[allow(clippy::too_many_arguments)]
    pub async fn store_response(
        conn: &mut PgConn,
        tenant_id: TenantId,
        operation: &str,
        idempotency_key: &str,
        request_fingerprint: &[u8],
        status: i32,
        content_type: Option<&str>,
        body: &[u8],
    ) -> DbResult<usize> {
        use crate::schema::api_idempotency_key::dsl as k;
        diesel::update(
            k::api_idempotency_key
                .filter(k::tenant_id.eq(tenant_id))
                .filter(k::operation.eq(operation))
                .filter(k::idempotency_key.eq(idempotency_key))
                .filter(k::request_fingerprint.eq(request_fingerprint)),
        )
        .set((
            k::response_status.eq(status),
            k::response_content_type.eq(content_type),
            k::response_body.eq(body),
        ))
        .execute(conn)
        .await
        .attach("Error while storing an idempotent response")
        .into_db_result()
    }
}

impl ApiIdempotencyKeyRow {
    /// Takes over a claim made before `claimed_before` that the call holding it left without a
    /// response (its process died): true when this call now holds it, restarted at `now`; false
    /// when another took it first or the first call answered meanwhile.
    pub async fn take_over_stale(
        conn: &mut PgConn,
        tenant_id: TenantId,
        operation: &str,
        idempotency_key: &str,
        request_fingerprint: &[u8],
        claimed_before: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> DbResult<bool> {
        use crate::schema::api_idempotency_key::dsl as k;
        let taken = diesel::update(
            k::api_idempotency_key
                .filter(k::tenant_id.eq(tenant_id))
                .filter(k::operation.eq(operation))
                .filter(k::idempotency_key.eq(idempotency_key))
                .filter(k::request_fingerprint.eq(request_fingerprint))
                .filter(k::response_status.is_null())
                .filter(k::created_at.lt(claimed_before)),
        )
        .set(k::created_at.eq(now))
        .execute(conn)
        .await
        .attach("Error while taking over a stale idempotency key")
        .into_db_result()?;
        Ok(taken == 1)
    }
}

impl ApiIdempotencyKeyRow {
    /// Forgets at most `limit` keys of `operation` claimed before `cutoff`, oldest first: one
    /// bounded DELETE, so retention never holds a long lock on the table.
    pub async fn purge_older_than(
        conn: &mut PgConn,
        operation: &str,
        cutoff: DateTime<Utc>,
        limit: i64,
    ) -> DbResult<usize> {
        let query = diesel::sql_query(
            "DELETE FROM api_idempotency_key WHERE (tenant_id, operation, idempotency_key) IN \
             (SELECT tenant_id, operation, idempotency_key FROM api_idempotency_key \
              WHERE operation = $3 AND created_at < $1 ORDER BY created_at LIMIT $2)",
        )
        .bind::<sql_types::Timestamptz, _>(cutoff)
        .bind::<sql_types::BigInt, _>(limit)
        .bind::<sql_types::Text, _>(operation);

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while purging idempotency keys")
            .into_db_result()
    }
}
