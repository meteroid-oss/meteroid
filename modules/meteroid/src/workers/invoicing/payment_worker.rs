use crate::adapters::stripe::Stripe;
use crate::workers::metrics::record_call;
use crate::{errors, singletons};
use common_utils::timed::TimedExt;
use error_stack::Result;
use fang::{AsyncQueueable, AsyncRunnable, Deserialize, FangError, Scheduled, Serialize};
use meteroid_store::{domain, Store};

const BATCH_SIZE: usize = 100;
const MAX_CONCURRENT_REQUESTS: usize = 10;

#[derive(Serialize, Deserialize)]
#[serde(crate = "fang::serde")]
pub struct PaymentWorker;

#[async_trait::async_trait]
#[typetag::serde]
impl AsyncRunnable for PaymentWorker {
    #[tracing::instrument(skip(self, _queue))]
    async fn run(&self, _queue: &mut dyn AsyncQueueable) -> core::result::Result<(), FangError> {
        payment_worker(singletons::get_store().await, Stripe::get())
            .timed(|res, elapsed| record_call("payment", res, elapsed))
            .await
            .map_err(|err| {
                log::error!("Error in payment worker: {}", err);
                FangError {
                    description: err.to_string(),
                }
            })
    }

    fn cron(&self) -> Option<Scheduled> {
        let expression = "0 4/10 * * * * *"; // every 10 minutes
        Some(Scheduled::CronPattern(expression.to_string()))
    }

    fn uniq(&self) -> bool {
        true
    }

    fn max_retries(&self) -> i32 {
        0
    }
}

#[tracing::instrument(skip_all)]
async fn payment_worker(store: &Store, stripe_adapter: &Stripe) -> Result<(), errors::WorkerError> {
    // let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));

    // let mut tasks = Vec::new();
    //
    // let mut last_processed_id = None;

    // TODO separate controllers & workers (workers => consumers, controllers => producers)
    // loop {
    //     // all invoices that are finalized & not fully paid, with a payment method, and where payment_method.auto_collect
    //     let paginated_vec = store
    //         .list_invoices_to_bill(
    //             5,
    //             CursorPaginationRequest {
    //                 limit: Some(BATCH_SIZE as u32),
    //                 cursor: last_processed_id,
    //             },
    //         )
    //         .await
    //         .change_context(errors::WorkerError::DatabaseError)?;
    //
    //     for invoice in paginated_vec.items.into_iter() {
    //         let permit = semaphore
    //             .clone()
    //             .acquire_owned()
    //             .await
    //             .change_context(errors::WorkerError::DatabaseError)?;
    //
    //         let store = store.clone();
    //         let stripe_adapter = stripe_adapter.clone();
    //
    //         let task = tokio::spawn(async move {
    //             let _permit = permit; // Moves permit into the async block
    //
    //             let payment_result = payment_invoice(&invoice, &stripe_adapter, &store).await;
    //
    //             match payment_result {
    //                 Ok(_) => {
    //                     let res = store
    //                         .invoice_payment_success(invoice.id, invoice.tenant_id)
    //                         .await;
    //
    //                     if let Err(e) = res {
    //                         log::error!(
    //                             "Failed to mark as payment_success invoice with id {} : {}",
    //                             &invoice.id,
    //                             e
    //                         )
    //                     }
    //                 }
    //                 Err(e) => {
    //                     let res = store
    //                         .invoice_payment_error(
    //                             invoice.id,
    //                             invoice.tenant_id,
    //                             e.to_string().as_str(),
    //                         )
    //                         .await;
    //
    //                     if let Err(e) = res {
    //                         log::error!(
    //                             "Failed to mark as payment_error invoice with id {} : {}",
    //                             &invoice.id,
    //                             e
    //                         )
    //                     }
    //                 }
    //             }
    //
    //             //  drop(_permit) should not be necessary, TODO validate
    //         });
    //         tasks.push(task);
    //     }
    //
    //     last_processed_id = paginated_vec.next_cursor;
    //
    //     if paginated_vec.next_cursor.is_none() {
    //         break;
    //     }
    // }

    // join_all(tasks).await;

    Ok(())
}

#[tracing::instrument(skip_all)]
async fn payment_invoice(
    invoice: &domain::Invoice,
    stripe_adapter: &Stripe,
    store: &Store,
) -> Result<(), errors::WorkerError> {
    // # 1- we resolve payment method for subscription

    // TODO
    // So subscription has a selected payment method (or does it ?)
    // But if a customer update its payment ùmethod, we need to change all subscriptions I guess
    // => should we just use the "payment_method_type" instead ? => probably, let's start with that

    // TODO subscription_id is optional. Because there are one-off invoice.
    // For one-off invoice, what's the process ? We create the transaction directly ?
    // Also, what about the first payment ?
    // or should we associate a payment_method to the invoice, ex: when finalizing ?
    // if let Some(subscription_id) = invoice.subscription_id {
    //     // we get the payment_method_type of the subscription
    //
    //     // we get the default_payment_method of the customer
    //
    //     store.get_subscription_details()
    //     // store.resolve_payment_method  => but was already resolved in query probably ?
    // }

    // - create a pending transaction

    // - send to stripe

    // follow up will be via webhook AFAIK.

    // we check the subscription mode
    // - IF AUTO (aka if defined payment method) & if collect = true
    // - then do we collect then payment, or payment then collect ?
    // I would try collecting, then send with a pay button TODO check

    // - IF MANUAL
    // we send by email (put in kafka)

    // TODO should we consider that this worker is only responsible for issuing (mail) ?
    // and we have another worker for billing via PP ?
    // ex: ready_to_payment or date TODO

    //

    // match invoice.payment_provider {
    //     PaymentProviderEnum::Stripe => {
    //         let customer = store
    //             .find_customer_by_id(invoice.customer_id, invoice.tenant_id)
    //             .await
    //             .change_context(errors::WorkerError::DatabaseError)?;
    //
    //         let api_key = store
    //             .find_provider_config(PaymentProviderEnum::Stripe, invoice.tenant_id)
    //             .await
    //             .change_context(errors::WorkerError::DatabaseError)?
    //             .api_security
    //             .api_key;
    //
    //         stripe_adapter
    //             .send_invoice(invoice, &customer, SecretString::new(api_key))
    //             .await
    //             .change_context(errors::WorkerError::ProviderError)?;
    //
    //         Ok(())
    //     }
    //     PaymentProviderEnum::Manual => {
    //         log::warn!("Invoice has Manual provider so shouldn't be picked-up by payment_worker");
    //         Ok(())
    //     }
    // }

    todo!()
}
