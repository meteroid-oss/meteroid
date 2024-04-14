/*
    Goal : We want to compute invoice amount at regular (configurable) interval, until the invoice is finalized (when it will be computed again with the final data).

*/
use std::sync::Arc;

use common_repository::Pool;
use meteroid_repository as db;

use crate::{compute::InvoiceEngine, errors};

use crate::compute::clients::usage::MeteringUsageClient;
use crate::repo::{get_pool, get_store};
use crate::workers::clients::metering::MeteringClient;
use crate::workers::metrics::record_call;
use common_utils::timed::TimedExt;
use error_stack::{Result, ResultExt};
use fang::{AsyncQueueable, AsyncRunnable, Deserialize, FangError, Scheduled, Serialize};
use futures::{future::join_all, stream::StreamExt};

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

#[derive(Serialize, Deserialize)]
#[serde(crate = "fang::serde")]
pub struct PriceWorker;

#[async_trait::async_trait]
#[typetag::serde]
impl AsyncRunnable for PriceWorker {
    #[tracing::instrument(skip_all)]
    async fn run(&self, _queue: &mut dyn AsyncQueueable) -> core::result::Result<(), FangError> {
        price_worker(
            get_pool().clone(),
            get_store().clone(),
            MeteringClient::get().clone(),
        )
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
pub async fn price_worker(
    db_pool: Pool,
    store: Store,
    metering_client: MeteringClient,
) -> Result<(), errors::WorkerError> {
    // fetch all invoice not finalized/voided and not updated since > 1h

    let connection = db_pool
        .get()
        .await
        .change_context(errors::WorkerError::DatabaseError)?;

    let mut outdated_invoices_query = db::invoices::get_outdated_invoices();
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));

    let query_service_client = metering_client.queries;

    let compute_service = Arc::new(InvoiceEngine::new(
        Arc::new(MeteringUsageClient::new(query_service_client)),
        Arc::new(store.clone()),
    ));

    let invoices_iter = outdated_invoices_query
        .bind(&connection)
        .iter()
        .await
        .change_context(errors::WorkerError::DatabaseError)?;

    let mut invoices_stream = Box::pin(invoices_iter);

    let mut tasks = Vec::new();
    // TODO optimize (semaphore + parallelism)
    while let Some(invoice_res) = invoices_stream.next().await {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .change_context(errors::WorkerError::DatabaseError)?;
        match invoice_res {
            Ok(invoice) => {
                let compute_service_clone: Arc<InvoiceEngine> = compute_service.clone();
                let store = store.clone();

                let connection = db_pool
                    .get()
                    .await
                    .change_context(errors::WorkerError::DatabaseError)?;

                let task = tokio::spawn(async move {
                    let _permit = permit; // Moves permit into the async block
                    let result = shared::update_invoice_line_items(
                        &invoice,
                        &compute_service_clone,
                        &connection,
                        store.clone(),
                    )
                    .await;
                    if let Err(e) = result {
                        // TODO this will retry, but we need to track/alert
                        log::error!("Failed to process invoice with id {} : {}", &invoice.id, e)
                    }

                    //  drop(_permit) should not be necessary, TODO validate
                });
                tasks.push(task);
            }
            Err(e) => {
                //TODO
                log::error!("Error while streaming invoice: {}", e)
            }
        }
    }

    join_all(tasks).await;

    Ok(())
}
