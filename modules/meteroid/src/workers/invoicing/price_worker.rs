/*
    Goal : We want to compute invoice amount at regular (configurable) interval, until the invoice is finalized (when it will be computed again with the final data).

*/
use std::sync::Arc;

use crate::{errors, singletons};

use crate::workers::metrics::record_call;
use common_utils::timed::TimedExt;
use error_stack::{Result, ResultExt};
use fang::{AsyncQueueable, AsyncRunnable, Deserialize, FangError, Scheduled, Serialize};
use futures::future::join_all;

use meteroid_store::domain::{CursorPaginationRequest, Invoice, InvoiceLinesPatch};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::Store;
use tokio::sync::Semaphore;

use super::shared;

/*
We stream open invoices whose price has not been updated in the last hour/period, ordered by last update date ASC
We compute the invoice amount
We update the invoice with the new amount (ONLY IF status is not finalized/voided) & update the date

*/
// let semaphore = Semaphore::new(MAX_CONCURRENT_REQUESTS);

const MAX_CONCURRENT_REQUESTS: usize = 10;

const BATCH_SIZE: usize = 100;

#[derive(Serialize, Deserialize)]
#[serde(crate = "fang::serde")]
pub struct PriceWorker;

#[async_trait::async_trait]
#[typetag::serde]
impl AsyncRunnable for PriceWorker {
    #[tracing::instrument(skip_all)]
    async fn run(&self, _queue: &mut dyn AsyncQueueable) -> core::result::Result<(), FangError> {
        price_worker(singletons::get_store().await)
            .timed(|res, elapsed| record_call("price", res, elapsed))
            .await
            .map_err(|err| {
                log::error!("Error in price worker: {}", err);
                FangError {
                    description: err.to_string(),
                }
            })
    }

    fn uniq(&self) -> bool {
        true
    }

    fn cron(&self) -> Option<Scheduled> {
        let expression = "0 2/10 * * * * *"; // every 10 minutes
        Some(Scheduled::CronPattern(expression.to_string()))
    }

    fn max_retries(&self) -> i32 {
        0
    }
}

#[tracing::instrument(skip_all)]
pub async fn price_worker(store: &Store) -> Result<(), errors::WorkerError> {
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));

    let mut tasks = Vec::new();

    let mut last_processed_id = None;

    // TODO optimize (semaphore + parallelism)
    loop {
        let paginated_vec = store
            .list_outdated_invoices(CursorPaginationRequest {
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

                let lines_result = update_invoice_lines(&invoice, store).await;

                if let Err(e) = lines_result {
                    // TODO this will retry, but we need to track/alert
                    log::error!(
                        "Failed to update lines for invoice with id {} : {}",
                        &invoice.id,
                        e
                    )
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

async fn update_invoice_lines(invoice: &Invoice, store: Store) -> Result<(), errors::WorkerError> {
    let lines = shared::get_invoice_lines(&invoice, store.clone()).await?;

    store
        .update_invoice_lines(
            invoice.id,
            invoice.tenant_id,
            InvoiceLinesPatch::from_invoice_and_lines(invoice, lines),
        )
        .await
        .change_context(errors::WorkerError::DatabaseError)
}
