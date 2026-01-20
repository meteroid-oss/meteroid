use crate::constants::OSS_API;
use crate::svix::SvixOps;
use meteroid_store::repositories::historical_rates::HistoricalRatesInterface;
use std::sync::Arc;
use svix::api::Svix;
use tap::TapFallible;

mod historical_rates;

const BASE_CURRENCY: &str = "USD";
const BASE_DATE: chrono::NaiveDate =
    chrono::NaiveDate::from_ymd_opt(2025, 1, 1).expect("Invalid base date constant in bootstrap");

pub async fn bootstrap_once(
    store: meteroid_store::Store,
    svix: Option<Arc<Svix>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // register svix event types
    if let Some(svix) = svix {
        svix.import_open_api_event_types(include_str!("../../../../spec/api/v1/openapi.json"))
            .await
            .tap_err(|err| {
                log::error!("Failed to import Svix event types: {}", err);
            })?;
    }

    // check if we need to setup historical rates
    let result = store
        .get_historical_rate(BASE_CURRENCY, BASE_CURRENCY, BASE_DATE)
        .await?;
    if result.is_none() {
        let parquet_file =
            historical_rates::fetch_parquet_file(&format!("{OSS_API}/historical-rates")).await?;
        let rates = historical_rates::read_parquet_bytes_to_exchange_rates(&parquet_file)?;
        log::info!("Inserting historical rates...");
        store.create_historical_rates_from_usd(rates).await?;
    }

    Ok(())
}
