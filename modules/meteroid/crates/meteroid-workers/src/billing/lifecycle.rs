use crate::utils::{backoff_duration, sleep_with_jitter};
use meteroid_store::Services;
use std::sync::Arc;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);
const BUSY_POLL: Duration = Duration::from_millis(500);
const IDLE_POLL: Duration = Duration::from_secs(5);

pub async fn run_worker(service: Arc<Services>) {
    let mut consecutive_errors: u32 = 0;

    loop {
        match tokio::time::timeout(TIMEOUT, service.get_and_process_cycle_transitions()).await {
            Ok(Ok(result)) => {
                consecutive_errors = 0;
                if result.processed > 0 {
                    log::debug!(
                        "Subscription lifecycle worker processed {} items",
                        result.processed
                    );
                }
                // Backfilling subscriptions advance one cycle per poll, so keep polling fast
                // while there is work and only back off once a poll comes back empty.
                if result.processed == 0 {
                    sleep_with_jitter(IDLE_POLL).await;
                } else if !result.has_more {
                    sleep_with_jitter(BUSY_POLL).await;
                }
            }
            Ok(Err(err)) => {
                consecutive_errors += 1;
                let backoff = backoff_duration(consecutive_errors);
                log::error!("Subscription lifecycle encountered error: {err:?}");
                sleep_with_jitter(backoff).await;
            }
            Err(_) => {
                consecutive_errors += 1;
                let backoff = backoff_duration(consecutive_errors);
                log::error!(
                    "Subscription lifecycle timed out after {} seconds",
                    TIMEOUT.as_secs()
                );
                sleep_with_jitter(backoff).await;
            }
        }
    }
}
