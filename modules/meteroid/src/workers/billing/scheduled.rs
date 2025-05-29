use meteroid_store::Services;
use std::sync::Arc;

pub async fn run_worker(service: Arc<Services>) {
    loop {
        // TODO drop that processor_id, makes no sense
        match service.get_and_process_due_events().await {
            Ok(processed) => {
                if processed > 0 {
                    log::debug!("Scheduled-events worker processed {} items", processed);
                } else {
                    // No items to process, sleep a bit before checking again
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
            Err(err) => {
                log::error!("Scheduled-events worker encountered error: {:?}", err);
                // Short delay before retrying after an error
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }
    }
}
