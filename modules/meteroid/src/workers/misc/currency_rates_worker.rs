use crate::api::services::utils::uuid_gen;
use crate::errors;
use crate::repo::get_pool;

use crate::services::currency_rates::{CurrencyRatesService, OpenexchangeRatesService};
use crate::workers::metrics::record_call;
use common_utils::timed::TimedExt;
use cornucopia_async::Params;
use deadpool_postgres::Pool;
use error_stack::{Result, ResultExt};
use fang::{AsyncQueueable, AsyncRunnable, Deserialize, FangError, Scheduled, Serialize};
use meteroid_repository as db;
use meteroid_repository::rates::InsertRatesParams;

#[derive(Serialize, Deserialize)]
#[serde(crate = "fang::serde")]
pub struct CurrencyRatesWorker;

#[async_trait::async_trait]
#[typetag::serde]
impl AsyncRunnable for CurrencyRatesWorker {
    #[tracing::instrument(skip(self, _queue))]
    async fn run(&self, _queue: &mut dyn AsyncQueueable) -> core::result::Result<(), FangError> {
        let pool = get_pool();
        currency_rates_worker(pool, OpenexchangeRatesService::get())
            .timed(|res, elapsed| record_call("issue", res, elapsed))
            .await
            .map_err(|err| {
                log::error!("Error in currency rates worker: {}", err);
                FangError {
                    description: err.to_string(),
                }
            })
    }

    fn cron(&self) -> Option<Scheduled> {
        let expression = "0 0 0/12 * * * *"; // twice a day
        Some(Scheduled::CronPattern(expression.to_string()))
    }

    fn uniq(&self) -> bool {
        true
    }

    fn max_retries(&self) -> i32 {
        1
    }
}

#[tracing::instrument(skip_all)]
async fn currency_rates_worker(
    pool: &Pool,
    currency_rates_service: &dyn CurrencyRatesService,
) -> Result<(), errors::WorkerError> {
    let conn = pool
        .get()
        .await
        .change_context(errors::WorkerError::DatabaseError)?;

    let rates = currency_rates_service
        .fetch_latest_exchange_rates()
        .await
        .change_context(errors::WorkerError::CurrencyRatesUpdateError)?;

    let rates_json = serde_json::to_value(&rates.rates)
        .change_context(errors::WorkerError::CurrencyRatesUpdateError)?;

    let date = time::OffsetDateTime::from_unix_timestamp(rates.timestamp as i64)
        .change_context(errors::WorkerError::CurrencyRatesUpdateError)?
        .date();

    // we insert or update the rates. bi table usd values are updated via a trigger
    db::rates::insert_rates()
        .params(
            &conn,
            &InsertRatesParams {
                id: uuid_gen::v7(),
                date,
                rates: rates_json,
            },
        )
        .one()
        .await
        .change_context(errors::WorkerError::CurrencyRatesUpdateError)?;

    Ok(())
}
