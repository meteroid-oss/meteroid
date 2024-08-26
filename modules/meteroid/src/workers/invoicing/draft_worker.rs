use crate::workers::metrics::record_call;
use crate::{errors, singletons};
use chrono::NaiveDate;

use common_utils::timed::*;

use error_stack::{Result, ResultExt};
use fang::{AsyncQueueable, AsyncRunnable, Deserialize, FangError, Scheduled, Serialize};

use common_eventbus::Event;

use meteroid_store::domain::CursorPaginationRequest;
use meteroid_store::repositories::subscriptions::subscription_to_draft;
use meteroid_store::repositories::{CustomersInterface, InvoiceInterface, SubscriptionInterface};
use meteroid_store::Store;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;

const BATCH_SIZE: usize = 100;

#[derive(Serialize, Deserialize)]
#[serde(crate = "fang::serde")]
pub struct DraftWorker;

#[async_trait::async_trait]
#[typetag::serde]
impl AsyncRunnable for DraftWorker {
    #[tracing::instrument(skip_all)]
    async fn run(&self, _queue: &mut dyn AsyncQueueable) -> core::result::Result<(), FangError> {
        log::info!("Running draft worker");
        draft_worker(
            singletons::get_store().await,
            chrono::Utc::now().naive_utc().date(),
        )
            .timed(|res, elapsed| record_call("draft", res, elapsed))
            .await
            .map_err(|err| {
                log::error!("Error in draft worker: {:?}", err);
                FangError {
                    description: err.to_string(),
                }
            })
    }

    fn uniq(&self) -> bool {
        true
    }

    fn cron(&self) -> Option<Scheduled> {
        let expression = "0 0 0/1 * * * *"; // every hour
        Some(Scheduled::CronPattern(expression.to_string()))
    }

    fn max_retries(&self) -> i32 {
        0
    }
}

#[tracing::instrument(skip_all)]
pub async fn draft_worker(store: &Store, today: NaiveDate) -> Result<(), errors::WorkerError> {
    let mut last_processed_id = None;

    loop {
        let paginated_vec = store
            .list_subscription_invoice_candidates(
                today,
                CursorPaginationRequest {
                    limit: Some(BATCH_SIZE as u32),
                    cursor: last_processed_id,
                },
            )
            .await
            .change_context(errors::WorkerError::DatabaseError)?;

        if paginated_vec.items.is_empty() {
            break;
        }

        let customer_ids = paginated_vec
            .items
            .iter()
            .map(|x| x.customer_id)
            .collect::<Vec<_>>();

        let customers = &store
            .list_customers_by_ids(customer_ids)
            .await
            .change_context(errors::WorkerError::DatabaseError)?;


        let invoicing_entity_ids = customers
            .iter()
            .map(|x| x.invoicing_entity_id)
            .collect::<std::collections::HashSet<_>>();

        let invoicing_entities = store
            .list_invoicing_entities_by_ids(invoicing_entity_ids.into_iter().collect())
            .await
            .change_context(errors::WorkerError::DatabaseError)?;

        let params = paginated_vec
            .items
            .iter()
            .map(|x| {
                let cust = customers
                    .iter()
                    .find(|c| c.id == x.customer_id)
                    .ok_or(errors::WorkerError::DatabaseError)?;

                let invoicing_entity = invoicing_entities
                    .iter()
                    .find(|c| c.id == cust.invoicing_entity_id)
                    .ok_or(errors::WorkerError::DatabaseError)?;

                subscription_to_draft(x, cust, invoicing_entity).change_context(errors::WorkerError::DatabaseError)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .collect::<Vec<_>>();

        log::debug!("Creating {} draft invoices", params.len());

        let inserted = store
            .insert_invoice_batch(params)
            .await
            .change_context(errors::WorkerError::DatabaseError)?;

        last_processed_id = paginated_vec.next_cursor;

        for inv in &inserted {
            let _ = store
                .eventbus
                .publish(Event::invoice_created(inv.id, inv.tenant_id))
                .await;
        }

        if paginated_vec.next_cursor.is_none() {
            break;
        }
    }

    Ok(())
}
