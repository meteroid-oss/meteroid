use crate::config::SvixConfig;
use crate::constants::OSS_API;
use crate::svix::SvixOps;
use meteroid_store::repositories::historical_rates::HistoricalRatesInterface;
use std::sync::Arc;
use tap::TapFallible;

mod historical_rates;

const BASE_CURRENCY: &str = "USD";
const BASE_DATE: chrono::NaiveDate =
    chrono::NaiveDate::from_ymd_opt(2025, 1, 1).expect("Invalid base date constant in bootstrap");

pub async fn bootstrap_once(
    store: meteroid_store::Store,
    svix: Option<Arc<dyn SvixOps>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // register svix event types
    if let Some(svix) = svix {
        svix.import_open_api_event_types(include_str!("../../../../spec/api/v1/openapi.json"))
            .await
            .tap_err(|err| {
                tracing::error!("Failed to import Svix event types: {}", err);
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
        tracing::info!("Inserting historical rates...");
        store.create_historical_rates_from_usd(rates).await?;
    }

    Ok(())
}

async fn verify_operational_webhook_registered(svix: &Arc<dyn SvixOps>, expected_url_suffix: &str) {
    match svix.list_operational_webhook_endpoints().await {
        Ok(endpoints) if endpoints.is_empty() => {
            tracing::warn!(
                "SVIX_OPERATIONAL_WEBHOOK_SECRET is set but no operational webhook endpoints \
                 are registered on Svix. Register one pointing at {expected_url_suffix}."
            );
        }
        Ok(endpoints) => {
            let matched = endpoints
                .iter()
                .any(|e| e.url.contains(expected_url_suffix) && e.disabled != Some(true));
            if matched {
                tracing::info!("Svix operational webhook registration verified");
            } else {
                tracing::warn!(
                    "SVIX_OPERATIONAL_WEBHOOK_SECRET is set but no registered endpoint URL \
                     contains `{expected_url_suffix}`. Op-webhook invalidation will not work."
                );
            }
        }
        Err(e) => {
            // Self-hosted Svix tokens may not have list permission; non-fatal.
            tracing::warn!("Could not verify Svix operational webhook registration: {e:?}");
        }
    }
}

/// Surfaces the two misconfigurations that silently break op-webhook invalidation:
/// op-secret set without Redis, and op-secret set without a matching endpoint on Svix.
pub async fn verify_svix_setup(
    svix_config: &SvixConfig,
    rest_api_external_url: &str,
    svix: Option<&Arc<dyn SvixOps>>,
    redis_available: bool,
) {
    if svix_config.operational_webhook_secret.is_none() {
        return;
    }

    if !redis_available {
        tracing::warn!(
            "SVIX_OPERATIONAL_WEBHOOK_SECRET is set but no Redis is configured — \
             endpoint cache is a no-op and invalidation has no effect."
        );
    }

    if let Some(svix) = svix {
        let trimmed = rest_api_external_url
            .trim_end_matches('/')
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let expected_suffix = format!(
            "{trimmed}{}",
            crate::api_rest::svix_operational::OP_WEBHOOK_PATH
        );
        verify_operational_webhook_registered(svix, &expected_suffix).await;
    }
}
