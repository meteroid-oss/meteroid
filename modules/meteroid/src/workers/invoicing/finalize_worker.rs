use std::sync::Arc;

use crate::{errors, singletons};

use common_utils::timed::TimedExt;
use error_stack::{Result, ResultExt};
use fang::{AsyncQueueable, AsyncRunnable, Deserialize, FangError, Scheduled, Serialize};
use futures::future::join_all;
use meteroid_store::domain::CursorPaginationRequest;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::Store;
use tokio::sync::Semaphore;

use crate::workers::metrics::record_call;

const BATCH_SIZE: usize = 100;
const MAX_CONCURRENT_REQUESTS: usize = 10;

#[derive(Serialize, Deserialize)]
#[serde(crate = "fang::serde")]
pub struct FinalizeWorker;

#[async_trait::async_trait]
#[typetag::serde]
impl AsyncRunnable for FinalizeWorker {
    #[tracing::instrument(skip_all)]
    async fn run(&self, _queue: &mut dyn AsyncQueueable) -> core::result::Result<(), FangError> {
        finalize_worker(singletons::get_store().await)
            .timed(|res, elapsed| record_call("finalize", res, elapsed))
            .await
            .map_err(|err| {
                log::error!("Error in finalize worker: {}", err);
                FangError {
                    description: err.to_string(),
                }
            })
    }

    fn uniq(&self) -> bool {
        true
    }

    fn cron(&self) -> Option<Scheduled> {
        let expression = "0 3/10 * * * * *"; // every 10 minutes
        Some(Scheduled::CronPattern(expression.to_string()))
    }

    fn max_retries(&self) -> i32 {
        0
    }
}

#[tracing::instrument(skip_all)]
pub async fn finalize_worker(store: &Store) -> Result<(), errors::WorkerError> {
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));

    let mut tasks = Vec::new();

    let mut last_processed_id = None;

    // TODO optimize (semaphore + parallelism)
    loop {
        let paginated_vec = store
            .list_invoices_to_finalize(CursorPaginationRequest {
                limit: Some(BATCH_SIZE as u32),
                cursor: last_processed_id,
            })
            .await
            .change_context(errors::WorkerError::DatabaseError)?;

        for invoice in paginated_vec.items.into_iter() {
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .change_context(errors::WorkerError::DatabaseError)?;

            let store = store.clone();

            let task = tokio::spawn(async move {
                let _permit = permit; // Moves permit into the async block

                let lines_result = store
                    .finalize_invoice(invoice.id, invoice.tenant_id)
                    .await
                    .change_context(errors::WorkerError::DatabaseError);

                if let Err(e) = lines_result {
                    // TODO this will retry, but we need to track/alert
                    log::error!("Failed to finalize invoice with id {} : {}", &invoice.id, e)
                }

                //  drop(_permit) should not be necessary, TODO validate
            });
            tasks.push(task);
        }

        last_processed_id = paginated_vec.next_cursor;

        if paginated_vec.next_cursor.is_none() {
            break;
        }
    }

    join_all(tasks).await;

    Ok(())
}
