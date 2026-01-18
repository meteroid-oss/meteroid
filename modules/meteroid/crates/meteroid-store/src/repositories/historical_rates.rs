use crate::domain::historical_rates::{
    HistoricalRate, HistoricalRatesFromUsd, HistoricalRatesFromUsdNew,
};
use crate::store::PgConn;
use crate::{Store, StoreResult};
use cached::proc_macro::cached;
use cached::proc_macro::once;
use chrono::NaiveDate;
use diesel_models::historical_rates_from_usd::{
    HistoricalRatesFromUsdRow, HistoricalRatesFromUsdRowNew,
};
use std::time::Duration;

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

    async fn latest_rate(
        &self,
        from_currency: &str,
        to_currency: &str,
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
            .map(std::convert::TryInto::try_into)
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
            .map(|rate| get_mapped_rates_for_currency(from_currency, to_currency, rate))
    }

    async fn latest_rate(
        &self,
        from_currency: &str,
        to_currency: &str,
    ) -> StoreResult<Option<HistoricalRate>> {
        let mut conn = self.get_conn().await?;

        get_latest_rate_from_usd_cached(&mut conn)
            .await
            .map(|rate| get_mapped_rates_for_currency(from_currency, to_currency, rate))
    }
}

#[cached(
    result = true,
    size = 10,
    time = 300, // 5min
    key = "NaiveDate",
    convert = r#"{ date }"#
)]
pub async fn get_historical_rate_from_usd_by_date_cached(
    conn: &mut PgConn,
    date: NaiveDate,
) -> StoreResult<Option<HistoricalRatesFromUsd>> {
    HistoricalRatesFromUsdRow::get_by_date(date, conn)
        .await
        .map_err(Into::into)
        .and_then(|row| row.map(TryInto::try_into).transpose())
}

#[once(
    result = true,
    time = 300, // 5min
)]
async fn get_latest_rate_from_usd_cached(
    conn: &mut PgConn,
) -> StoreResult<Option<HistoricalRatesFromUsd>> {
    HistoricalRatesFromUsdRow::latest(conn)
        .await
        .map_err(Into::into)
        .and_then(|row| row.map(TryInto::try_into).transpose())
}

fn get_mapped_rates_for_currency(
    from_currency: &str,
    to_currency: &str,
    rate: Option<HistoricalRatesFromUsd>,
) -> Option<HistoricalRate> {
    rate.and_then(|rate| {
        rate.rates.get(from_currency).and_then(|from_rate| {
            rate.rates.get(to_currency).map(|to_rate| HistoricalRate {
                id: rate.id,
                date: rate.date,
                updated_at: rate.updated_at,
                from_currency: from_currency.to_string(),
                to_currency: to_currency.to_string(),
                rate: to_rate / from_rate,
            })
        })
    })
}

/// Transaction-compatible version of get_historical_rate.
/// Use this when you need to get a historical rate within an existing transaction.
pub async fn get_historical_rate_tx(
    conn: &mut PgConn,
    from_currency: &str,
    to_currency: &str,
    date: NaiveDate,
) -> StoreResult<Option<HistoricalRate>> {
    let rate = get_historical_rate_from_usd_by_date_cached(conn, date).await?;
    Ok(get_mapped_rates_for_currency(
        from_currency,
        to_currency,
        rate,
    ))
}

/// Transaction-compatible version of latest_rate.
/// Use this when you need to get the latest exchange rate within an existing transaction.
pub async fn latest_rate_tx(
    conn: &mut PgConn,
    from_currency: &str,
    to_currency: &str,
) -> StoreResult<Option<HistoricalRate>> {
    let rate = get_latest_rate_from_usd_cached(conn).await?;
    Ok(get_mapped_rates_for_currency(
        from_currency,
        to_currency,
        rate,
    ))
}
