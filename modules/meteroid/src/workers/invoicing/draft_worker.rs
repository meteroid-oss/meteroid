use crate::workers::metrics::record_call;
use crate::{errors, singletons};
use chrono::NaiveDate;

use common_utils::timed::*;

use error_stack::{Result, ResultExt};
use fang::{AsyncQueueable, AsyncRunnable, Deserialize, FangError, Scheduled, Serialize};

use common_eventbus::Event;

use meteroid_store::domain::enums::{
    BillingPeriodEnum, InvoiceStatusEnum, InvoiceType, InvoicingProviderEnum,
};
use meteroid_store::domain::{
    BillingConfig, CursorPaginationRequest, SubscriptionInvoiceCandidate,
};
use meteroid_store::repositories::{CustomersInterface, InvoiceInterface, SubscriptionInterface};
use meteroid_store::Store;

const BATCH_SIZE: usize = 100;

#[derive(Serialize, Deserialize)]
#[serde(crate = "fang::serde")]
pub struct DraftWorker;

#[async_trait::async_trait]
#[typetag::serde]
impl AsyncRunnable for DraftWorker {
    #[tracing::instrument(skip_all)]
    async fn run(&self, _queue: &mut dyn AsyncQueueable) -> core::result::Result<(), FangError> {
        draft_worker(
            singletons::get_store().await,
            chrono::Utc::now().naive_utc().date(),
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

        let params = paginated_vec
            .items
            .iter()
            .map(|x| {
                let cust_bill_cfg = customers
                    .iter()
                    .find(|c| c.id == x.customer_id)
                    .and_then(|x| x.billing_config.as_ref());

                subscription_to_draft(x, cust_bill_cfg)
            })
            .collect::<Result<Vec<Option<_>>, _>>()?
            .into_iter()
            .flatten()
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

#[tracing::instrument]
fn subscription_to_draft(
    subscription: &SubscriptionInvoiceCandidate,
    cust_bill_cfg: Option<&BillingConfig>,
) -> Result<Option<meteroid_store::domain::invoices::InvoiceNew>, errors::WorkerError> {
    let billing_start_date = subscription.billing_start_date;
    let billing_day = subscription.billing_day as u32;

    let mut billing_periods: Vec<BillingPeriodEnum> = subscription
        .periods
        .iter()
        .filter_map(|a| a.as_billing_period_opt())
        .collect();
    billing_periods.sort();
    let period = billing_periods.first();

    if period.is_none() {
        return Ok(None);
    }

    let period = meteroid_store::utils::periods::calculate_period_range(
        billing_start_date,
        billing_day,
        0,
        period.unwrap(),
    );

    let invoicing_provider = match cust_bill_cfg {
        Some(BillingConfig::Stripe(_)) => InvoicingProviderEnum::Stripe,
        None => InvoicingProviderEnum::Manual,
    };

    let invoice = meteroid_store::domain::invoices::InvoiceNew {
        tenant_id: subscription.tenant_id,
        customer_id: subscription.customer_id,
        subscription_id: Some(subscription.id),
        amount_cents: None, // TODO let's calculate here (just skipping the usage)
        plan_version_id: Some(subscription.plan_version_id),
        invoice_type: InvoiceType::Recurring,
        currency: subscription.currency.clone(),
        days_until_due: Some(subscription.net_terms),
        external_invoice_id: None,
        invoice_id: None, // TODO
        invoicing_provider,
        line_items: serde_json::Value::Null, // TODO
        issued: false,
        issue_attempts: 0,
        last_issue_attempt_at: None,
        last_issue_error: None,
        data_updated_at: None,
        status: InvoiceStatusEnum::Draft,
        external_status: None,
        invoice_date: period.end,
        finalized_at: None,
    };

    Ok(Some(invoice))
}
