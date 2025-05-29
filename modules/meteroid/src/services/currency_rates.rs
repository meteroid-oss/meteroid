use crate::constants::OSS_API;

use async_trait::async_trait;

use itertools::Itertools;
use serde::Deserialize;
use std::collections::BTreeMap;
use strum::{Display, EnumIter, EnumString, IntoEnumIterator};
use thiserror::Error;

static OPEN_EXCHANGES_RATES_SERVICE: std::sync::OnceLock<OpenexchangeRatesService> =
    std::sync::OnceLock::new();

#[derive(Error, Debug)]
pub enum CurrencyRatesError {
    #[error("Failed to fetch exchange rates")]
    FetchFailed,
    #[error("Failed to parse exchange rates")]
    ParseFailed,
    #[error("Rates must be USD-based, but found {0}")]
    InvalidBase(String),
}

#[async_trait]
pub trait CurrencyRatesService: Send + Sync + 'static {
    async fn fetch_latest_exchange_rates(&self) -> Result<ExchangeRates, CurrencyRatesError>;
}

#[derive(Deserialize, Debug)]
pub struct ExchangeRates {
    pub base: String,
    pub rates: BTreeMap<String, f32>,
    pub timestamp: u64,
}

pub struct OpenexchangeRatesService {
    api_key: Option<String>,
    client: reqwest::Client,
}

impl OpenexchangeRatesService {
    pub fn new(client: reqwest::Client, api_key: Option<String>) -> Self {
        Self { api_key, client }
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
    async fn fetch(&self, base: Currency) -> Result<ExchangeRates, CurrencyRatesError> {
        let symbols = Currency::iter().filter(|x| *x != base).format(",");

        let url = match self.api_key.as_ref() {
            None =>
            // This api is provided for testing/development purposes only, with no warranties.
            // In production, use openexchangerates or equivalent.
            {
                format!("{}/rates/{}?symbols={}", OSS_API, base, symbols)
            }
            Some(api_key) => format!(
                "https://openexchangerates.org/api/latest.json?app_id={}&base={}&symbols={}",
                api_key, base, symbols
            ),
        };
        let response = self
            .client
            .get(url)
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
        let rates = self.fetch(Currency::USD).await?;

        if rates.base != Currency::USD.to_string() {
            return Err(CurrencyRatesError::InvalidBase(rates.base.to_string()));
        }

        Ok(rates)
    }
}

// todo: we should support only currencies we have conversion rates for. Introduce a centralized currency enum so we can use it in multiple places or generate from it enums in different layers
#[derive(Clone, Debug, Copy, EnumString, Display, EnumIter, PartialOrd, PartialEq)]
#[allow(clippy::upper_case_acronyms)]
pub enum Currency {
    AED,
    AFN,
    ALL,
    AMD,
    ANG,
    AOA,
    ARS,
    AUD,
    AWG,
    AZN,
    BAM,
    BBD,
    BDT,
    BGN,
    BHD,
    BIF,
    BMD,
    BND,
    BOB,
    BRL,
    BSD,
    BTN,
    BWP,
    BYN,
    BZD,
    CAD,
    CDF,
    CHF,
    CLP,
    CNH,
    CNY,
    COP,
    CRC,
    CUC,
    CUP,
    CVE,
    CZK,
    DJF,
    DKK,
    DOP,
    DZD,
    EGP,
    ERN,
    ETB,
    EUR,
    FJD,
    FKP,
    GBP,
    GEL,
    GHS,
    GIP,
    GMD,
    GNF,
    GTQ,
    GYD,
    HKD,
    HNL,
    HRK,
    HTG,
    HUF,
    IDR,
    ILS,
    INR,
    IQD,
    IRR,
    ISK,
    JMD,
    JOD,
    JPY,
    KES,
    KGS,
    KHR,
    KMF,
    KPW,
    KRW,
    KWD,
    KYD,
    KZT,
    LAK,
    LBP,
    LKR,
    LRD,
    LSL,
    LYD,
    MAD,
    MDL,
    MGA,
    MKD,
    MMK,
    MNT,
    MOP,
    MRU,
    MUR,
    MVR,
    MWK,
    MXN,
    MYR,
    MZN,
    NAD,
    NGN,
    NIO,
    NOK,
    NPR,
    NZD,
    OMR,
    PAB,
    PEN,
    PGK,
    PHP,
    PKR,
    PLN,
    PYG,
    QAR,
    RON,
    RSD,
    RUB,
    RWF,
    SAR,
    SBD,
    SCR,
    SDG,
    SEK,
    SGD,
    SHP,
    SLL,
    SOS,
    SRD,
    SSP,
    STD,
    STN,
    SVC,
    SYP,
    SZL,
    THB,
    TJS,
    TMT,
    TND,
    TOP,
    TRY,
    TTD,
    TWD,
    TZS,
    UAH,
    UGX,
    USD,
    UYU,
    UZS,
    VES,
    VND,
    VUV,
    WST,
    XAF,
    XCD,
    XOF,
    XPF,
    YER,
    ZAR,
    ZMW,
    ZWL,
}
