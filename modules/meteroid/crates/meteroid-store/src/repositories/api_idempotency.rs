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
                return Err(key_reused());
            }
            Ok(ClaimOutcome::Existing)
        }
    }
}

/// The refusal of a key already used with another request: a conflict, marked so API v2 answers
/// it as `idempotency_key_reused`.
pub(crate) fn key_reused() -> Report<StoreError> {
    Report::new(StoreError::Conflict(
        "this Idempotency-Key was already used with a different request".to_owned(),
    ))
    .attach_opaque(crate::errors::IdempotencyKeyReused)
}

/// What a v2 write finds under its Idempotency-Key (ADR-0013).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiResponseClaim {
    /// Nobody held the key: this call runs, then stores its response or releases the key.
    Fresh,
    /// The same request already ran: its response, replayed as it was answered (its content type
    /// absent on a response kept before it was).
    Replay {
        status: u16,
        content_type: Option<String>,
        body: Vec<u8>,
    },
    /// The same request is still running.
    InFlight,
    /// The key was used with another request.
    Mismatch,
}

/// How long a v2 claim stays unanswered before a retry takes it over: far past any request's run,
/// so only a claim whose process died (a crash, a killed pod) is taken.
pub const IN_FLIGHT_STALE_MINUTES: i64 = 10;

/// The responses of API v2 writes, kept under their Idempotency-Key in `api_idempotency_key`
/// (the claim and fingerprint v1 writes use), so a retry replays the first answer.
#[async_trait::async_trait]
pub trait ApiIdempotentResponsesInterface {
    /// Claims the key for this call, or says what holds it. The claim commits at once.
    async fn claim_api_response(
        &self,
        tenant_id: TenantId,
        identity: &ApiIdempotency,
    ) -> StoreResult<ApiResponseClaim>;

    async fn store_api_response(
        &self,
        tenant_id: TenantId,
        identity: &ApiIdempotency,
        status: u16,
        content_type: Option<&str>,
        body: &[u8],
    ) -> StoreResult<()>;

    /// Forgets a claim whose call failed, so it can be retried.
    async fn release_api_response(
        &self,
        tenant_id: TenantId,
        identity: &ApiIdempotency,
    ) -> StoreResult<()>;

    /// Forgets at most `limit` keys of `operation` claimed before `cutoff`, oldest first, in one
    /// bounded DELETE; returns how many went. Only the v2 middleware's replay records are purged:
    /// the store's own keys (v1's, and the v2 writes that derive a payment's or a grant's id from
    /// their key) are kept, so a late retry replays rather than colliding on that id.
    async fn purge_api_idempotency_keys(
        &self,
        operation: &str,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> StoreResult<usize>;
}

#[async_trait::async_trait]
impl ApiIdempotentResponsesInterface for crate::Store {
    async fn claim_api_response(
        &self,
        tenant_id: TenantId,
        identity: &ApiIdempotency,
    ) -> StoreResult<ApiResponseClaim> {
        use diesel_models::api_idempotency::ApiIdempotencyKeyRow;

        let mut conn = self.get_conn().await?;
        let claim = ApiIdempotencyKeyRowNew {
            tenant_id,
            operation: identity.operation.to_owned(),
            idempotency_key: identity.key.clone(),
            request_fingerprint: identity.fingerprint.to_vec(),
        };
        match claim
            .claim_in_tx(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        {
            ApiIdempotencyClaim::Fresh => Ok(ApiResponseClaim::Fresh),
            ApiIdempotencyClaim::Existing(existing) => {
                if existing.request_fingerprint != claim.request_fingerprint {
                    return Ok(ApiResponseClaim::Mismatch);
                }
                let stored = ApiIdempotencyKeyRow::stored_response(
                    &mut conn,
                    tenant_id,
                    identity.operation,
                    &identity.key,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                match stored {
                    Some(response) => Ok(ApiResponseClaim::Replay {
                        status: u16::try_from(response.status).unwrap_or(200),
                        content_type: response.content_type,
                        body: response.body,
                    }),
                    None => {
                        // Claim times are real time, as the retention job reads them.
                        let now = common_clock::wall();
                        // Unanswered long past any request's run: its process died. Whether it
                        // wrote is unknown, as it would be once retention forgot the key; the
                        // writes that move money replay their own store key (`claim_tx`).
                        let taken = ApiIdempotencyKeyRow::take_over_stale(
                            &mut conn,
                            tenant_id,
                            identity.operation,
                            &identity.key,
                            &identity.fingerprint,
                            now - chrono::Duration::minutes(IN_FLIGHT_STALE_MINUTES),
                            now,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                        Ok(if taken {
                            ApiResponseClaim::Fresh
                        } else {
                            ApiResponseClaim::InFlight
                        })
                    }
                }
            }
        }
    }

    async fn store_api_response(
        &self,
        tenant_id: TenantId,
        identity: &ApiIdempotency,
        status: u16,
        content_type: Option<&str>,
        body: &[u8],
    ) -> StoreResult<()> {
        use diesel_models::api_idempotency::ApiIdempotencyKeyRow;

        let mut conn = self.get_conn().await?;
        ApiIdempotencyKeyRow::store_response(
            &mut conn,
            tenant_id,
            identity.operation,
            &identity.key,
            &identity.fingerprint,
            i32::from(status),
            content_type,
            body,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        Ok(())
    }

    async fn release_api_response(
        &self,
        tenant_id: TenantId,
        identity: &ApiIdempotency,
    ) -> StoreResult<()> {
        use diesel_models::api_idempotency::ApiIdempotencyKeyRow;

        let mut conn = self.get_conn().await?;
        ApiIdempotencyKeyRow::release(
            &mut conn,
            tenant_id,
            identity.operation,
            &identity.key,
            &identity.fingerprint,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        Ok(())
    }

    async fn purge_api_idempotency_keys(
        &self,
        operation: &str,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> StoreResult<usize> {
        use diesel_models::api_idempotency::ApiIdempotencyKeyRow;

        let mut conn = self.get_conn().await?;
        ApiIdempotencyKeyRow::purge_older_than(&mut conn, operation, cutoff, limit.max(1))
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }
}
