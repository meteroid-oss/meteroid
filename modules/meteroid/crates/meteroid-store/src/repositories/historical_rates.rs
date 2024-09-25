use crate::domain::historical_rates::{
    HistoricalRate, HistoricalRatesFromUsd, HistoricalRatesFromUsdNew,
};
use crate::{Store, StoreResult};
use chrono::NaiveDate;
use diesel_models::historical_rates_from_usd::{
    HistoricalRatesFromUsdRow, HistoricalRatesFromUsdRowNew,
};

#[async_trait::async_trait]
pub trait HistoricalRatesInterface {
    async fn create_historical_rate_from_usd(
        &self,
        rate: HistoricalRatesFromUsdNew,
    ) -> StoreResult<HistoricalRatesFromUsd>;

    async fn get_historical_rate_from_usd_by_date(
        &self,
        date: chrono::NaiveDate,
    ) -> StoreResult<Option<HistoricalRatesFromUsd>>;

    async fn get_historical_rate(
        &self,
        from_currency: &str,
        to_currency: &str,
        date: chrono::NaiveDate,
    ) -> StoreResult<Option<HistoricalRate>>;
}

#[async_trait::async_trait]
impl HistoricalRatesInterface for Store {
    async fn create_historical_rate_from_usd(
        &self,
        rate: HistoricalRatesFromUsdNew,
    ) -> StoreResult<HistoricalRatesFromUsd> {
        let mut conn = self.get_conn().await?;

        let insertable: HistoricalRatesFromUsdRowNew = rate.try_into()?;

        insertable
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn get_historical_rate_from_usd_by_date(
        &self,
        date: chrono::NaiveDate,
    ) -> StoreResult<Option<HistoricalRatesFromUsd>> {
        let mut conn = self.get_conn().await?;

        HistoricalRatesFromUsdRow::get_by_date(date, &mut conn)
            .await
            .map_err(Into::into)
            .and_then(|row| row.map(TryInto::try_into).transpose())
    }

    async fn get_historical_rate(
        &self,
        from_currency: &str,
        to_currency: &str,
        date: NaiveDate,
    ) -> StoreResult<Option<HistoricalRate>> {
        self.get_historical_rate_from_usd_by_date(date)
            .await
            .map(|rate| {
                rate.and_then(|rate| {
                    rate.rates.get(from_currency).and_then(|from_rate| {
                        rate.rates.get(to_currency).map(|to_rate| HistoricalRate {
                            id: rate.id,
                            date: rate.date,
                            from_currency: from_currency.to_string(),
                            to_currency: to_currency.to_string(),
                            rate: to_rate / from_rate,
                        })
                    })
                })
            })
    }
}
