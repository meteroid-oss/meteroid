use crate::StoreResult;
use crate::domain::credit::ApiIdempotency;
use crate::errors::StoreError;
use crate::store::PgConn;
use common_domain::ids::TenantId;
use diesel_models::api_idempotency::ApiIdempotencyKeyRowNew;
use diesel_models::query::api_idempotency::ApiIdempotencyClaim;
use error_stack::Report;

/// Whether this call is the one that claimed the key inside its own transaction, or a replay of
/// an earlier call whose transaction already committed.
pub(crate) enum ClaimOutcome {
    Fresh,
    Existing,
}

/// Claimed in the caller's transaction before any other lock: a committed claim proves the write
/// ran, so a replay skips it; a concurrent duplicate waits on the key's row.
pub(crate) async fn claim_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    identity: &ApiIdempotency,
) -> StoreResult<ClaimOutcome> {
    let claim = ApiIdempotencyKeyRowNew {
        tenant_id,
        operation: identity.operation.to_owned(),
        idempotency_key: identity.key.clone(),
        request_fingerprint: identity.fingerprint.to_vec(),
    };
    match claim.claim_in_tx(conn).await? {
        ApiIdempotencyClaim::Fresh => Ok(ClaimOutcome::Fresh),
        ApiIdempotencyClaim::Existing(existing) => {
            if existing.request_fingerprint != claim.request_fingerprint {
                return Err(Report::new(StoreError::Conflict(
                    "this Idempotency-Key was already used with a different request".to_owned(),
                )));
            }
            Ok(ClaimOutcome::Existing)
        }
    }
}
