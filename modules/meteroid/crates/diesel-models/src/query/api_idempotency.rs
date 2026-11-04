use crate::api_idempotency::{ApiIdempotencyKeyRow, ApiIdempotencyKeyRowNew};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::TenantId;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
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
