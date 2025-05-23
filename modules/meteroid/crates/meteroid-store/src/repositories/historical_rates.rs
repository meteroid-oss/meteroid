use crate::domain::historical_rates::{
    HistoricalRate, HistoricalRatesFromUsd, HistoricalRatesFromUsdNew,
};
use crate::store::PgConn;
use crate::{Store, StoreResult};
use cached::proc_macro::cached;
use chrono::NaiveDate;
use diesel_models::historical_rates_from_usd::{
    HistoricalRatesFromUsdRow, HistoricalRatesFromUsdRowNew,
};

#[async_trait::async_trait]
pub trait HistoricalRatesInterface {
    async fn create_historical_rates_from_usd(
        &self,
        rates: Vec<HistoricalRatesFromUsdNew>,
    ) -> StoreResult<()>;

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
    async fn create_historical_rates_from_usd(
        &self,
        rates: Vec<HistoricalRatesFromUsdNew>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let batch = rates
            .into_iter()
            .map(|s| s.try_into())
            .collect::<Result<Vec<_>, _>>()?;

        HistoricalRatesFromUsdRowNew::insert_batch(&mut conn, batch)
            .await
            .map_err(Into::into)
    }

    async fn get_historical_rate_from_usd_by_date(
        &self,
        date: chrono::NaiveDate,
    ) -> StoreResult<Option<HistoricalRatesFromUsd>> {
        let mut conn = self.get_conn().await?;

        get_historical_rate_from_usd_by_date_cached(&mut conn, date).await
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

#[cached(
    result = true,
    size = 10,
    time = 300, // 5min
    key = "NaiveDate",
    convert = r#"{ date }"#
)]
async fn get_historical_rate_from_usd_by_date_cached(
    conn: &mut PgConn,
    date: NaiveDate,
) -> StoreResult<Option<HistoricalRatesFromUsd>> {
    HistoricalRatesFromUsdRow::get_by_date(date, conn)
        .await
        .map_err(Into::into)
        .and_then(|row| row.map(TryInto::try_into).transpose())
}
