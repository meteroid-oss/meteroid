use crate::workers::pgmq::sleep_with_jitter;
use meteroid_store::Services;
use std::sync::Arc;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);

pub async fn run_worker(service: Arc<Services>) {
    loop {
        match tokio::time::timeout(TIMEOUT, service.get_and_process_cycle_transitions()).await {
            Ok(Ok(result)) => {
                if result.processed > 0 {
                    log::debug!(
                        "Subscription lifecycle worker processed {} items",
                        result.processed
                    );
                }
                // Only sleep if we didn't hit the batch limit (no more work pending)
                if !result.has_more {
                    sleep_with_jitter(tokio::time::Duration::from_millis(100)).await;
                }
            }
            Ok(Err(err)) => {
                log::error!("Subscription lifecycle encountered error: {err:?}");
                // Short delay before retrying after an error
                sleep_with_jitter(tokio::time::Duration::from_secs(1)).await;
            }
            Err(_) => {
                log::error!(
                    "Subscription lifecycle timed out after {} seconds",
                    TIMEOUT.as_secs()
                );
                sleep_with_jitter(tokio::time::Duration::from_secs(1)).await;
            }
        }
    }
}
