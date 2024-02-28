use meteroid_repository as db;

use crate::api::services::utils::uuid_gen;
use crate::constants::{OSS_API, SUPPORTED_CURRENCIES};
use crate::repo::get_pool;
use async_trait::async_trait;
use cornucopia_async::Params;
use deadpool_postgres::Pool;
use meteroid_repository::rates::InsertRatesParams;
use serde::Deserialize;
use std::collections::BTreeMap;
use thiserror::Error;

static OPEN_EXCHANGES_RATES_SERVICE: std::sync::OnceLock<OpenexchangeRatesService> =
    std::sync::OnceLock::new();

#[derive(Error, Debug)]
enum CurrencyRatesError {
    #[error("Failed to save exchange rates")]
    DbError,
    #[error("Failed to fetch exchange rates")]
    FetchFailed,
    #[error("Failed to parse exchange rates")]
    ParseFailed,
    #[error("Rates must be USD-based, but found {0}")]
    InvalidBase(String),
}

#[async_trait]
pub(crate) trait CurrencyRatesService: Send + Sync + 'static {
    async fn fetch_latest_exchange_rates(&self) -> Result<ExchangeRates, CurrencyRatesError>;
}

#[derive(Deserialize, Debug)]
pub struct ExchangeRates {
    pub base: String,
    pub rates: BTreeMap<String, f32>,
    pub timestamp: u64,
}

pub struct OpenexchangeRatesService {
    openex_api_key: Option<String>,
    client: reqwest::Client,
}

impl OpenexchangeRatesService {
    fn new(client: reqwest::Client, openex_api_key: Option<String>) -> Self {
        Self {
            openex_api_key,
            client,
        }
    }

    pub fn get() -> &'static Self {
        OPEN_EXCHANGES_RATES_SERVICE.get_or_init(|| {
            let config = crate::config::Config::get();
            OpenexchangeRatesService::new(
                reqwest::Client::new(),
                config.openexchangerates_api_key.clone(),
            )
        })
    }

    #[tracing::instrument(skip(self))]
    async fn fetch_from_openexchangerates(
        &self,
        api_key: String,
    ) -> Result<ExchangeRates, CurrencyRatesError> {
        let response = self
            .client
            .get(&format!(
                "https://openexchangerates.org/api/latest.json?app_id={}&base=USD&symbols={}",
                api_key,
                SUPPORTED_CURRENCIES.join(",")
            ))
            .send()
            .await
            .map_err(|_| CurrencyRatesError::FetchFailed)?;

        let rates = response
            .json::<ExchangeRates>()
            .await
            .map_err(|_| CurrencyRatesError::ParseFailed)?;

        Ok(rates)
    }

    /**
     * This api is provided for testing/development purposes only, with no warranties. In production, use openexchangerates or equivalent.
     */
    #[tracing::instrument(skip(self))]
    async fn fetch_from_cloud_fallback(&self) -> Result<ExchangeRates, CurrencyRatesError> {
        let response = self
            .client
            .get(&format!(
                "{}/rates/USD?symbols={}",
                OSS_API,
                SUPPORTED_CURRENCIES.join(",")
            ))
            .send()
            .await
            .map_err(|_| CurrencyRatesError::FetchFailed)?;

        let rates = response
            .json::<ExchangeRates>()
            .await
            .map_err(|_| CurrencyRatesError::ParseFailed)?;

        Ok(rates)
    }
}

#[async_trait]
impl CurrencyRatesService for OpenexchangeRatesService {
    #[tracing::instrument(skip_all)]
    async fn fetch_latest_exchange_rates(&self) -> Result<ExchangeRates, CurrencyRatesError> {
        // fetch from openexchangerates if an apikey is provided, else use the fallback
        let rates = if let Some(api_key) = &self.openex_api_key {
            self.fetch_from_openexchangerates(api_key.clone()).await?
        } else {
            self.fetch_from_cloud_fallback().await?
        };

        if rates.base != "USD" {
            return Err(CurrencyRatesError::InvalidBase(rates.base));
        }

        Ok(rates)
    }
}
