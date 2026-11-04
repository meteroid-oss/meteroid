use crate::utils::{backoff_duration, sleep_with_jitter};
use meteroid_store::Services;
use std::sync::Arc;
use std::time::{Duration, Instant};

const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);
const TIMEOUT: Duration = Duration::from_secs(10);

pub async fn run_worker(service: Arc<Services>) {
    let mut last_cleanup = Instant::now();
    let mut consecutive_errors: u32 = 0;

    loop {
        match tokio::time::timeout(TIMEOUT, service.get_and_process_due_events()).await {
            Ok(Ok(processed)) => {
                consecutive_errors = 0;
                if processed > 0 {
                    log::debug!("Scheduled-events worker processed {processed} items");
                } else {
                    // Run cleanup periodically (handles events stuck in Processing state)
                    if last_cleanup.elapsed() > CLEANUP_INTERVAL {
                        match tokio::time::timeout(
                            TIMEOUT,
                            service.cleanup_timeout_scheduled_events(),
                        )
                        .await
                        {
                            Ok(Ok(_)) => {}
                            Ok(Err(e)) => {
                                log::warn!("Scheduled-events cleanup failed: {e:?}");
                            }
                            Err(_) => {
                                log::warn!(
                                    "Scheduled-events cleanup timed out after {} seconds",
                                    TIMEOUT.as_secs()
                                );
                            }
                        }
                        last_cleanup = Instant::now();
                    }
                    // No items to process, sleep a bit before checking again
                    sleep_with_jitter(Duration::from_millis(500)).await;
                }
            }
            Ok(Err(err)) => {
                consecutive_errors += 1;
                let backoff = backoff_duration(consecutive_errors);
                log::error!("Scheduled-events worker encountered error: {err:?}");
                sleep_with_jitter(backoff).await;
            }
            Err(_) => {
                consecutive_errors += 1;
                let backoff = backoff_duration(consecutive_errors);
                log::error!(
                    "Scheduled-events worker timed out after {} seconds",
                    TIMEOUT.as_secs()
                );
                sleep_with_jitter(backoff).await;
            }
        }
    }
}
