use crate::errors;
use error_stack::{Report, ResultExt};
use meteroid_store::Store;
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use std::sync::Arc;
use std::time::Duration;

const CLEANUP_INTERVAL_SECS: u64 = 3600; // 1 hour
const RETENTION_DAYS: u32 = 3;

pub async fn run_checkout_session_cleanup_worker(store: Arc<Store>) {
    loop {
        // Simple jitter for easy concurrency
        let jitter_duration = Duration::from_secs(rand::random::<u64>() % 60);

        match cleanup_checkout_sessions(&store).await {
            Ok((expired, deleted)) => {
                if expired > 0 || deleted > 0 {
                    log::info!(
                        "Checkout session cleanup: marked {} sessions as expired, deleted {} old sessions",
                        expired,
                        deleted
                    );
                }
            }
            Err(err) => {
                log::error!("Checkout session cleanup worker encountered error: {err:?}");
            }
        }

        tokio::time::sleep(Duration::from_secs(CLEANUP_INTERVAL_SECS) + jitter_duration).await;
    }
}

async fn cleanup_checkout_sessions(
    store: &Arc<Store>,
) -> Result<(usize, usize), Report<errors::WorkerError>> {
    // Mark expired sessions
    let expired_count = store
        .expire_sessions()
        .await
        .change_context(errors::WorkerError::CheckoutSessionCleanupError)?;

    // Delete old completed/expired sessions
    let deleted_count = store
        .cleanup_old_sessions(RETENTION_DAYS)
        .await
        .change_context(errors::WorkerError::CheckoutSessionCleanupError)?;

    Ok((expired_count, deleted_count))
}
