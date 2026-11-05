use meteroid_store::Store;
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;
use std::sync::Arc;
use std::time::Duration;

const CLEANUP_INTERVAL_SECS: u64 = 24 * 3600;

pub async fn run_portal_token_cleanup_worker(store: Arc<Store>) {
    loop {
        let jitter_duration = Duration::from_secs(rand::random::<u64>() % 600);

        match store.purge_stale_portal_tokens().await {
            Ok(0) => {}
            Ok(deleted) => log::info!("Portal token cleanup: deleted {deleted} stale tokens"),
            Err(err) => log::error!("Portal token cleanup failed: {err:?}"),
        }

        tokio::time::sleep(Duration::from_secs(CLEANUP_INTERVAL_SECS) + jitter_duration).await;
    }
}
