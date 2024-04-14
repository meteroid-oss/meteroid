use crate::workers::metrics::record_call;
use crate::{errors, repo::get_pool};
use common_utils::timed::*;
use cornucopia_async::Params;
use deadpool_postgres::Pool;
use error_stack::{Result, ResultExt};
use fang::{AsyncQueueable, AsyncRunnable, Deserialize, FangError, Scheduled, Serialize};
use futures::StreamExt;
use meteroid_repository as db;
use std::ops::Deref;
use time::Date;

use crate::eventbus::{Event, EventBus, EventBusStatic};
use meteroid_repository::subscriptions::SubscriptionToInvoice;
use meteroid_store::domain::enums::{BillingPeriodEnum};

const BATCH_SIZE: usize = 100;

#[derive(Serialize, Deserialize)]
#[serde(crate = "fang::serde")]
pub struct DraftWorker;

#[async_trait::async_trait]
#[typetag::serde]
impl AsyncRunnable for DraftWorker {
    #[tracing::instrument(skip_all)]
    async fn run(&self, _queue: &mut dyn AsyncQueueable) -> core::result::Result<(), FangError> {
        let pool = get_pool();
        let eventbus = EventBusStatic::get().await;

        draft_worker(
            pool,
            eventbus.deref(),
            time::OffsetDateTime::now_utc().date(),
        )
        .timed(|res, elapsed| record_call("draft", res, elapsed))
        .await
        .map_err(|err| {
            log::error!("Error in draft worker: {}", err);
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
pub async fn draft_worker(
    pool: &Pool,
    eventbus: &dyn EventBus<Event>,
    today: Date,
) -> Result<(), errors::WorkerError> {
    let db_client = pool
        .get()
        .await
        .change_context(errors::WorkerError::DatabaseError)?;

    // we fetch all subscriptions that are active and that DO NOT HAVE an invoice where invoice_date > date
    // TODO this will not work if multiple opened invoices, we need a better way.
    // Should we just rely on the pending worker to create the next iteration. Else we should rely on the components
    let mut stmt = db::subscriptions::subscription_to_invoice_candidates();

    let stream = stmt
        .bind(&db_client, &today)
        .iter()
        .await
        .change_context(errors::WorkerError::DatabaseError)?;

    let chunks = stream.chunks(BATCH_SIZE);

    futures::pin_mut!(chunks);

    while let Some(batch_result) = chunks.next().await {
        let batch = batch_result
            .into_iter()
            .collect::<std::result::Result<Vec<_>, _>>()
            .change_context(errors::WorkerError::GrpcError)?;

        let mut mut_db_client = pool
            .get()
            .await
            .change_context(errors::WorkerError::DatabaseError)?;

        let transaction = mut_db_client
            .transaction()
            .await
            .change_context(errors::WorkerError::DatabaseError)?;

        let params = batch
            .iter()
            .map(|p| subscription_to_draft(p, today))
            .collect::<Result<Vec<Option<_>>, _>>()?
            .into_iter()
            .filter_map(|p| p)
            .collect::<Vec<_>>();

        log::debug!("Creating {} draft invoices", params.len());

        // cornucopia doesn't support batch insert (unless you UNNEST, but we'll move away from cornucopia soon anyway)
        for param in &params {
            db::invoices::create_invoice()
                .params(&transaction, param)
                .one()
                .await
                .change_context(errors::WorkerError::DatabaseError)?;
        }

        transaction
            .commit()
            .await
            .change_context(errors::WorkerError::DatabaseError)?;

        for param in &params {
            let _ = eventbus
                .publish(Event::invoice_created(param.id, param.tenant_id))
                .await;
        }
    }

    Ok(())
}

#[tracing::instrument]
fn subscription_to_draft(
    subscription: &SubscriptionToInvoice,
    today: Date,
) -> Result<Option<db::invoices::CreateInvoiceParams<String, serde_json::Value>>, errors::WorkerError>
{
    let billing_start_date =
        crate::mapping::common::date_to_chrono(subscription.billing_start_date)
            .change_context(errors::WorkerError::InvalidInput)?;
    let today = crate::mapping::common::date_to_chrono(today)
        .change_context(errors::WorkerError::InvalidInput)?;
    let billing_day = subscription.billing_day as u32;

    let periods = meteroid_store::utils::periods::calculate_periods_for_date(
        billing_start_date,
        billing_day,
        today,
        &BillingPeriodEnum::Monthly,
    );

    match periods.advance {
        None => Ok(None),
        Some(period) => {
            let period_start_date = crate::mapping::common::chrono_to_date(period.end)
                .change_context(errors::WorkerError::InvalidInput)?;

            let params = db::invoices::CreateInvoiceParams {
                id: common_utils::uuid::v7(),
                invoicing_provider: db::InvoicingProviderEnum::STRIPE, // TODO
                status: db::InvoiceStatusEnum::DRAFT,
                invoice_date: period_start_date,
                tenant_id: subscription.tenant_id,
                customer_id: subscription.customer_id,
                subscription_id: subscription.subscription_id,
                plan_version_id: subscription.plan_version_id,
                currency: subscription.currency.clone(),
                days_until_due: subscription.net_terms,
                line_items: serde_json::Value::Null,
                amount_cents: None,
            };

            Ok(Some(params))
        }
    }
}
