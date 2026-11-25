//! Retention of `api_idempotency_key` (ADR-0013 §3): the v2 middleware's replay records (operation
//! `v2`) are kept for 24 hours, the window a retry replays its response in; afterwards the same key
//! runs the request again, and the store's own keys (kept forever, as v1's always were) replay the
//! writes that derive an id from it (a payment, a credit grant). Each run forgets one bounded
//! batch, oldest first.
use meteroid_store::Store;
use meteroid_store::repositories::ApiIdempotentResponsesInterface;
use std::sync::Arc;
use std::time::Duration;

/// The v2 idempotency middleware's operation (`meteroid_rest::v2::idempotency`).
const V2_REPLAY_OPERATION: &str = "v2";
const RETENTION_HOURS: i64 = 24;
const CLEANUP_INTERVAL_SECS: u64 = 10 * 60;
/// At most this many keys per run: 144 runs a day keep up with ~720k writes a day.
const PURGE_BATCH: i64 = 5_000;

pub async fn run_idempotency_key_cleanup_worker(store: Arc<Store>) {
    loop {
        let jitter_duration = Duration::from_secs(rand::random::<u64>() % 60);

        let cutoff = common_clock::wall() - chrono::Duration::hours(RETENTION_HOURS);
        match store
            .purge_api_idempotency_keys(V2_REPLAY_OPERATION, cutoff, PURGE_BATCH)
            .await
        {
            Ok(0) => {}
            Ok(deleted) => {
                log::info!("Idempotency key cleanup: deleted {deleted} keys older than {cutoff}")
            }
            Err(err) => log::error!("Idempotency key cleanup failed: {err:?}"),
        }

        tokio::time::sleep(Duration::from_secs(CLEANUP_INTERVAL_SECS) + jitter_duration).await;
    }
}
