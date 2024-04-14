use std::sync::Arc;

use common_repository::Pool;
use meteroid_repository as db;

use crate::{compute::InvoiceEngine, errors};

use crate::compute::clients::usage::MeteringUsageClient;
use crate::eventbus::{Event, EventBus, EventBusStatic};
use crate::repo::{get_pool, get_store};
use common_utils::timed::TimedExt;
use error_stack::{Result, ResultExt};
use fang::{AsyncQueueable, AsyncRunnable, Deserialize, FangError, Scheduled, Serialize};
use futures::{future::join_all, stream::StreamExt};
use meteroid_store::Store;
use tokio::sync::Semaphore;

use crate::workers::clients::metering::MeteringClient;
use crate::workers::metrics::record_call;

use super::shared;

const MAX_CONCURRENT_REQUESTS: usize = 10;

#[derive(Serialize, Deserialize)]
#[serde(crate = "fang::serde")]
pub struct FinalizeWorker;

#[async_trait::async_trait]
#[typetag::serde]
impl AsyncRunnable for FinalizeWorker {
    #[tracing::instrument(skip_all)]
    async fn run(&self, _queue: &mut dyn AsyncQueueable) -> core::result::Result<(), FangError> {
        let eventbus = EventBusStatic::get().await;
        finalize_worker(
            get_pool().clone(),
            MeteringClient::get().clone(),
            eventbus.clone(),
            get_store().clone(),
        )
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
pub async fn finalize_worker(
    db_pool: Pool,
    metering_client: MeteringClient,
    eventbus: Arc<dyn EventBus<Event>>,
    store: Store,
) -> Result<(), errors::WorkerError> {
    let connection = db_pool
        .get()
        .await
        .change_context(errors::WorkerError::DatabaseError)?;

    let mut invoices_query = db::invoices::get_invoices_to_finalize();

    let invoices_iter = invoices_query
        .bind(&connection)
        .iter()
        .await
        .change_context(errors::WorkerError::DatabaseError)?;

    let mut invoices_stream = Box::pin(invoices_iter);
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));

    let compute_service = Arc::new(InvoiceEngine::new(
        Arc::new(MeteringUsageClient::new(metering_client.queries)),
        Arc::new(store.clone()),
    ));

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

                let eventbus = eventbus.clone();

                let task = tokio::spawn(async move {
                    let _permit = permit; // Moves permit into the async block
                    let mut result = shared::update_invoice_line_items(
                        &invoice,
                        &compute_service_clone,
                        &connection,
                        store,
                    )
                        .await;

                    if result.is_ok() {
                        result = db::invoices::update_invoice_status()
                            .bind(&connection, &db::InvoiceStatusEnum::FINALIZED, &invoice.id)
                            .await
                            .change_context(errors::WorkerError::DatabaseError)
                            .map(|_| ());

                        let _ = eventbus
                            .publish(Event::invoice_finalized(
                                invoice.id.clone(),
                                invoice.tenant_id.clone(),
                            ))
                            .await;
                    }

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
