use crate::errors::StoreError;
use chrono::NaiveDate;
use error_stack::Report;
use std::collections::BTreeMap;
use uuid::Uuid;

pub struct HistoricalRatesFromUsd {
    pub id: Uuid,
    pub date: NaiveDate,
    pub rates: BTreeMap<String, f32>,
}

impl TryFrom<diesel_models::historical_rates_from_usd::HistoricalRatesFromUsd>
    for HistoricalRatesFromUsd
{
    type Error = Report<StoreError>;

    fn try_from(
        value: diesel_models::historical_rates_from_usd::HistoricalRatesFromUsd,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            date: value.date,
            rates: serde_json::from_value::<BTreeMap<String, f32>>(value.rates).map_err(|e| {
                StoreError::SerdeError("Failed to deserialize currency rates".to_string(), e)
            })?,
        })
    }
}

pub struct HistoricalRatesFromUsdNew {
    pub date: NaiveDate,
    pub rates: BTreeMap<String, f32>,
}

impl TryInto<diesel_models::historical_rates_from_usd::HistoricalRatesFromUsdNew>
    for HistoricalRatesFromUsdNew
{
    type Error = Report<StoreError>;

    fn try_into(
        self,
    ) -> Result<diesel_models::historical_rates_from_usd::HistoricalRatesFromUsdNew, Self::Error>
    {
        Ok(
            diesel_models::historical_rates_from_usd::HistoricalRatesFromUsdNew {
                id: Uuid::now_v7(),
                date: self.date,
                rates: serde_json::to_value::<BTreeMap<String, f32>>(self.rates).map_err(|e| {
                    StoreError::SerdeError("Failed to serialize currency rates".to_string(), e)
                })?,
            },
        )
    }
}
