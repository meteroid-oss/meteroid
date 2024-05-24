use crate::{errors, singletons};

use crate::services::currency_rates::{CurrencyRatesService, OpenexchangeRatesService};
use crate::workers::metrics::record_call;
use common_utils::timed::TimedExt;
use error_stack::{Result, ResultExt};
use fang::{AsyncQueueable, AsyncRunnable, Deserialize, FangError, Scheduled, Serialize};
use meteroid_store::domain::historical_rates::HistoricalRatesFromUsdNew;
use meteroid_store::repositories::historical_rates::HistoricalRatesInterface;
use meteroid_store::Store;

#[derive(Serialize, Deserialize)]
#[serde(crate = "fang::serde")]
pub struct CurrencyRatesWorker;

#[async_trait::async_trait]
#[typetag::serde]
impl AsyncRunnable for CurrencyRatesWorker {
    #[tracing::instrument(skip(self, _queue))]
    async fn run(&self, _queue: &mut dyn AsyncQueueable) -> core::result::Result<(), FangError> {
        let store = singletons::get_store().await;

        currency_rates_worker(store, OpenexchangeRatesService::get())
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
    store: &Store,
    currency_rates_service: &dyn CurrencyRatesService,
) -> Result<(), errors::WorkerError> {
    let rates = currency_rates_service
        .fetch_latest_exchange_rates()
        .await
        .change_context(errors::WorkerError::CurrencyRatesUpdateError)?;

    let date = chrono::DateTime::from_timestamp(rates.timestamp as i64, 0)
        .ok_or(errors::WorkerError::CurrencyRatesUpdateError)?
        .date_naive();

    // we insert or update the rates. bi table usd values are updated via a trigger
    store
        .create_historical_rate_from_usd(HistoricalRatesFromUsdNew {
            date,
            rates: rates.rates,
        })
        .await
        .change_context(errors::WorkerError::CurrencyRatesUpdateError)?;

    Ok(())
}
