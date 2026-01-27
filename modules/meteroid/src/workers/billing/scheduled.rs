use crate::workers::pgmq::sleep_with_jitter;
use meteroid_store::Services;
use std::sync::Arc;
use std::time::{Duration, Instant};

const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

pub async fn run_worker(service: Arc<Services>) {
    let mut last_cleanup = Instant::now();

    loop {
        match service.get_and_process_due_events().await {
            Ok(processed) => {
                if processed > 0 {
                    log::debug!("Scheduled-events worker processed {processed} items");
                } else {
                    // Run cleanup periodically (handles events stuck in Processing state)
                    if last_cleanup.elapsed() > CLEANUP_INTERVAL {
                        if let Err(e) = service.cleanup_timeout_scheduled_events().await {
                            log::warn!("Scheduled-events cleanup failed: {e:?}");
                        }
                        last_cleanup = Instant::now();
                    }
                    // No items to process, sleep a bit before checking again
                    sleep_with_jitter(tokio::time::Duration::from_millis(100)).await;
                }
            }
            Err(err) => {
                log::error!("Scheduled-events worker encountered error: {err:?}");
                // Short delay before retrying after an error
                sleep_with_jitter(tokio::time::Duration::from_secs(1)).await;
            }
        }
    }
}
