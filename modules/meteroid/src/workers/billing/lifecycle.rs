use meteroid_store::Services;
use std::sync::Arc;

pub async fn run_worker(service: Arc<Services>) {
    loop {
        match service.get_and_process_cycle_transitions().await {
            Ok(processed) => {
                if processed > 0 {
                    log::debug!(
                        "Subscription lifecycle worker processed {} items",
                        processed
                    );
                } else {
                    // No items to process, sleep a bit before checking again
                    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                }
            }
            Err(err) => {
                log::error!("Subscription lifecycle encountered error: {:?}", err);
                // Short delay before retrying after an error
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }
    }
}
