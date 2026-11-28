use crate::config::InstanceBootstrapConfig;
use meteroid_store::domain::historical_rates::HistoricalRatesFromUsdNew;
use meteroid_store::repositories::historical_rates::HistoricalRatesInterface;

mod historical_rates;
pub mod instance;

const BASE_CURRENCY: &str = "USD";
const BASE_DATE: chrono::NaiveDate =
    chrono::NaiveDate::from_ymd_opt(2025, 1, 1).expect("Invalid base date constant in bootstrap");

pub async fn bootstrap_once(
    store: meteroid_store::Store,
    instance_bootstrap: &InstanceBootstrapConfig,
    rates_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = store
        .get_historical_rate(BASE_CURRENCY, BASE_CURRENCY, BASE_DATE)
        .await?;
    if result.is_none()
        && let Some(rates) = fetch_historical_rates(rates_url).await
    {
        tracing::info!("Inserting historical rates...");
        store.create_historical_rates_from_usd(rates).await?;
    }

    instance::bootstrap_instance(&store, instance_bootstrap).await?;

    Ok(())
}

/// `None` when the rates API is unreachable: the backfill is retried on the next boot, and the
/// currency rates worker keeps the current rates up to date meanwhile.
async fn fetch_historical_rates(rates_url: &str) -> Option<Vec<HistoricalRatesFromUsdNew>> {
    let url = format!("{}/historical-rates", rates_url.trim_end_matches('/'));
    let rates = historical_rates::fetch_parquet_file(&url)
        .await
        .and_then(|bytes| historical_rates::read_parquet_bytes_to_exchange_rates(&bytes));
    match rates {
        Ok(rates) => Some(rates),
        Err(err) => {
            tracing::warn!(
                "Skipping the historical exchange rates backfill, it will be retried on the next \
                 boot: could not load {url} ({err:#}). Set METEROID_RATES_URL to a reachable \
                 rates API on offline or air-gapped installs."
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unreachable_rates_api_skips_the_backfill() {
        assert!(fetch_historical_rates("http://127.0.0.1:1").await.is_none());
    }

    #[tokio::test]
    async fn non_parquet_response_skips_the_backfill() {
        let app = axum::Router::new().route(
            "/historical-rates",
            axum::routing::get(|| async { "<html>captive portal</html>" }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });

        assert!(fetch_historical_rates(&url).await.is_none());
    }
}
