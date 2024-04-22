use crate::adapters::stripe::Stripe;
use crate::adapters::types::InvoicingAdapter;
use crate::workers::metrics::record_call;
use crate::{errors, singletons};
use common_utils::timed::TimedExt;
use cornucopia_async::Params;
use deadpool_postgres::Pool;
use error_stack::{Result, ResultExt};
use fang::{AsyncQueueable, AsyncRunnable, Deserialize, FangError, Scheduled, Serialize};
use meteroid_repository as db;
use meteroid_repository::invoices::UpdateInvoiceIssueErrorParams;
use meteroid_repository::InvoicingProviderEnum;
use meteroid_store::repositories::configs::ConfigsInterface;
use meteroid_store::Store;
use secrecy::SecretString;

#[derive(Serialize, Deserialize)]
#[serde(crate = "fang::serde")]
pub struct IssueWorker;

#[async_trait::async_trait]
#[typetag::serde]
impl AsyncRunnable for IssueWorker {
    #[tracing::instrument(skip(self, _queue))]
    async fn run(&self, _queue: &mut dyn AsyncQueueable) -> core::result::Result<(), FangError> {
        issue_worker(
            singletons::get_pool(),
            Stripe::get(),
            singletons::get_store(),
        )
        .timed(|res, elapsed| record_call("issue", res, elapsed))
        .await
        .map_err(|err| {
            log::error!("Error in issue worker: {}", err);
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
async fn issue_worker(
    pool: &Pool,
    stripe_adapter: &Stripe,
    store: &Store,
) -> Result<(), errors::WorkerError> {
    // fetch all invoices with issue=false and send to stripe

    let connection = pool
        .get()
        .await
        .change_context(errors::WorkerError::DatabaseError)?;

    let max_attempts = 5;

    // todo use streaming + batches
    let invoices = db::invoices::get_invoices_to_issue()
        .bind(&connection, &max_attempts)
        .all()
        .await
        .change_context(errors::WorkerError::DatabaseError)?;

    for invoice in invoices {
        let result = issue_invoice(&invoice, stripe_adapter, store, pool).await;

        let connection = pool
            .get()
            .await
            .change_context(errors::WorkerError::DatabaseError)?;

        match result {
            Ok(_) => {
                db::invoices::update_invoice_issue_success()
                    .params(
                        &connection,
                        &db::invoices::UpdateInvoiceIssueSuccessParams {
                            id: invoice.id,
                            issue_attempts: invoice.issue_attempts + 1,
                        },
                    )
                    .await
                    .change_context(errors::WorkerError::DatabaseError)?;
            }
            Err(err) => {
                let params = UpdateInvoiceIssueErrorParams {
                    issue_attempts: invoice.issue_attempts + 1,
                    last_issue_error: err.to_string(),
                    id: invoice.id,
                };

                db::invoices::update_invoice_issue_error()
                    .params(&connection, &params)
                    .await
                    .change_context(errors::WorkerError::DatabaseError)?;
            }
        }
    }

    Ok(())
}

#[tracing::instrument(skip_all)]
async fn issue_invoice(
    invoice: &db::invoices::Invoice,
    stripe_adapter: &Stripe,
    store: &Store,
    pool: &Pool,
) -> Result<(), errors::WorkerError> {
    match invoice.invoicing_provider {
        InvoicingProviderEnum::STRIPE => {
            let conn = pool
                .get()
                .await
                .change_context(errors::WorkerError::DatabaseError)?;

            let customer = db::customers::get_customer_by_id()
                .bind(&conn, &invoice.customer_id)
                .one()
                .await
                .change_context(errors::WorkerError::DatabaseError)?;

            let customer = crate::api::customers::mapping::customer::db_to_server(customer)
                .change_context(errors::WorkerError::DatabaseError)?;

            let api_key = store
                .find_provider_config(
                    meteroid_store::domain::enums::InvoicingProviderEnum::Stripe,
                    invoice.tenant_id,
                )
                .await
                .change_context(errors::WorkerError::DatabaseError)?
                .api_security
                .api_key;

            stripe_adapter
                .send_invoice(invoice, &customer, SecretString::new(api_key))
                .await
                .change_context(errors::WorkerError::ProviderError)?;

            Ok(())
        }
    }
}
