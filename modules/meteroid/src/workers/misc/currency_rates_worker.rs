use crate::errors;
use std::sync::Arc;
use std::time::Duration;

use crate::services::currency_rates::CurrencyRatesService;
use error_stack::{Result, ResultExt};
use meteroid_store::Store;
use meteroid_store::domain::historical_rates::HistoricalRatesFromUsdNew;
use meteroid_store::repositories::historical_rates::HistoricalRatesInterface;

pub async fn run_currency_rates_worker(
    store: &Arc<Store>,
    currency_rates_service: &Arc<dyn CurrencyRatesService>,
) {
    loop {
        // simple jitter for easy concurrency
        let jitter_duration = Duration::from_secs(rand::random::<u64>() % 60);

        match update_currency_rates(store, currency_rates_service).await {
            Ok(duration) => {
                tokio::time::sleep(duration + jitter_duration).await;
            }
            Err(err) => {
                log::error!("Currency rates worker encountered error: {:?}", err);
                tokio::time::sleep(Duration::from_secs(60) + jitter_duration).await;
            }
        }
    }
}

async fn update_currency_rates(
    store: &Arc<Store>,
    currency_rates_service: &Arc<dyn CurrencyRatesService>,
) -> Result<Duration, errors::WorkerError> {
    let rates = store
        .latest_rate("USD", "USD")
        .await
        .change_context(errors::WorkerError::CurrencyRatesUpdateError)?;

    let latest_timestamp = rates.as_ref().map(|r| r.updated_at);

    // we skip if it was updated in the past hour
    if let Some(latest_timestamp) = latest_timestamp {
        let now = chrono::Utc::now().naive_utc();
        let expected = now - chrono::Duration::hours(1);
        if latest_timestamp > expected {
            // we return the time to wait
            let duration = latest_timestamp
                .signed_duration_since(expected)
                .to_std()
                .unwrap_or(Duration::from_secs(3600));

            // wait at least 10 min
            let duration = std::cmp::max(duration, Duration::from_secs(360));
            return Ok(duration);
        }
    }

    let rates = currency_rates_service
        .fetch_latest_exchange_rates()
        .await
        .change_context(errors::WorkerError::CurrencyRatesUpdateError)?;

    let date = chrono::DateTime::from_timestamp(rates.timestamp as i64, 0)
        .ok_or(errors::WorkerError::CurrencyRatesUpdateError)?
        .date_naive();

    // we insert or update the rates. bi table usd values are updated via a trigger
    store
        .create_historical_rates_from_usd(vec![HistoricalRatesFromUsdNew {
            date,
            rates: rates.rates,
        }])
        .await
        .change_context(errors::WorkerError::CurrencyRatesUpdateError)?;

    Ok(Duration::from_secs(3600))
}
