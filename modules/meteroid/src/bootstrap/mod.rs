use crate::constants::OSS_API;
use meteroid_store::repositories::historical_rates::HistoricalRatesInterface;

mod historical_rates;

const BASE_CURRENCY: &str = "USD";
const BASE_DATE: chrono::NaiveDate =
    chrono::NaiveDate::from_ymd_opt(2025, 1, 1).expect("Invalid base date constant in bootstrap");

pub async fn bootstrap_once(
    store: meteroid_store::Store,
    services: meteroid_store::Services,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Err(err) = services.insert_webhook_out_event_types().await {
        log::error!("Failed to insert webhook out event types: {:?}", err)
    }

    // check if we need to setup historical rates
    let result = store
        .get_historical_rate(BASE_CURRENCY, BASE_CURRENCY, BASE_DATE)
        .await?;
    if result.is_none() {
        let parquet_file =
            historical_rates::fetch_parquet_file(&format!("{}/historical-rates", OSS_API)).await?;
        let rates = historical_rates::read_parquet_bytes_to_exchange_rates(&parquet_file)?;
        log::info!("Inserting historical rates...");
        store.create_historical_rates_from_usd(rates).await?;
    }

    Ok(())
}
