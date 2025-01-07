use crate::errors::StoreError;
use chrono::NaiveDate;
use diesel_models::historical_rates_from_usd::{
    HistoricalRatesFromUsdRow, HistoricalRatesFromUsdRowNew,
};
use error_stack::Report;
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct HistoricalRatesFromUsd {
    pub id: Uuid,
    pub date: NaiveDate,
    pub rates: BTreeMap<String, f32>,
}

pub struct HistoricalRate {
    pub id: Uuid,
    pub date: NaiveDate,
    pub from_currency: String,
    pub to_currency: String,
    pub rate: f32,
}

impl TryFrom<HistoricalRatesFromUsdRow> for HistoricalRatesFromUsd {
    type Error = Report<StoreError>;

    fn try_from(value: HistoricalRatesFromUsdRow) -> Result<Self, Self::Error> {
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

impl TryInto<HistoricalRatesFromUsdRowNew> for HistoricalRatesFromUsdNew {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<HistoricalRatesFromUsdRowNew, Self::Error> {
        Ok(HistoricalRatesFromUsdRowNew {
            id: Uuid::now_v7(),
            date: self.date,
            rates: serde_json::to_value::<BTreeMap<String, f32>>(self.rates).map_err(|e| {
                StoreError::SerdeError("Failed to serialize currency rates".to_string(), e)
            })?,
        })
    }
}
