use crate::domain::historical_rates::{
    HistoricalRate, HistoricalRatesFromUsd, HistoricalRatesFromUsdNew,
};
use crate::errors::StoreError;
use crate::store::PgConn;
use crate::{Store, StoreResult};
use cached::Cached;
use cached::proc_macro::cached;
use cached::proc_macro::once;
use chrono::NaiveDate;
use diesel_models::historical_rates_from_usd::{
    HistoricalRatesFromUsdRow, HistoricalRatesFromUsdRowNew,
};
use error_stack::Report;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::collections::{HashMap, HashSet};

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

    /// Resolve applicable historical rates for several dates in one call.
    /// Consults the per-date cache, then performs a single bounded DB lookup
    /// for the misses on one connection. Dates with no applicable rate are
    /// absent from the returned map.
    async fn find_historical_rates_by_dates(
        &self,
        dates: &[NaiveDate],
    ) -> StoreResult<HashMap<NaiveDate, HistoricalRatesFromUsd>>;

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

    async fn find_historical_rates_by_dates(
        &self,
        dates: &[NaiveDate],
    ) -> StoreResult<HashMap<NaiveDate, HistoricalRatesFromUsd>> {
        if dates.is_empty() {
            return Ok(HashMap::new());
        }

        let unique: Vec<NaiveDate> = dates
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let mut resolved: HashMap<NaiveDate, HistoricalRatesFromUsd> =
            HashMap::with_capacity(unique.len());
        let mut missing: Vec<NaiveDate> = Vec::new();

        {
            let mut cache = GET_HISTORICAL_RATE_FROM_USD_BY_DATE_CACHED.lock().await;
            for &d in &unique {
                match cache.cache_get(&d) {
                    Some(Some(rate)) => {
                        resolved.insert(d, rate.clone());
                    }
                    Some(None) => {}
                    None => missing.push(d),
                }
            }
        }

        if missing.is_empty() {
            return Ok(resolved);
        }

        let mut conn = self.get_conn().await?;
        let mut in_force: Vec<HistoricalRatesFromUsd> =
            HistoricalRatesFromUsdRow::list_in_force_on(&missing, &mut conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?;
        in_force.sort_by_key(|r| r.date);
        let mut fetched: Vec<(NaiveDate, Option<HistoricalRatesFromUsd>)> =
            Vec::with_capacity(missing.len());
        for d in &missing {
            let mapped = match in_force.partition_point(|r| r.date <= *d) {
                0 => None,
                n => Some(in_force[n - 1].clone()),
            };
            if let Some(ref m) = mapped {
                resolved.insert(*d, m.clone());
            }
            fetched.push((*d, mapped));
        }

        {
            let mut cache = GET_HISTORICAL_RATE_FROM_USD_BY_DATE_CACHED.lock().await;
            for (d, mapped) in fetched {
                cache.cache_set(d, mapped);
            }
        }

        Ok(resolved)
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

/// Clears all cached historical rates.
///
/// # Warning
/// This function is intended for **testing purposes only**. Calling this in production
/// will cause a temporary performance degradation as the cache needs to be repopulated.
///
/// This should be called in tests before each test that uses a fresh database,
/// to avoid FK constraint violations from cached UUIDs that don't exist in the new database.
#[doc(hidden)]
pub async fn clear_historical_rates_cache() {
    // Clear the #[cached] cache (TimedSizedCache wrapped in async Mutex)
    GET_HISTORICAL_RATE_FROM_USD_BY_DATE_CACHED
        .lock()
        .await
        .cache_clear();

    // Clear the #[once] cache (RwLock wrapped in Lazy)
    // The #[once] macro stores Option<(Instant, T)>, setting to None clears it
    *GET_LATEST_RATE_FROM_USD_CACHED.write().await = None;
}

/// Converts an amount from one currency to another using the latest exchange rate.
/// If the currencies are the same, returns the original amount unchanged.
pub async fn convert_currency(
    conn: &mut PgConn,
    amount_cents: i64,
    from_currency: &str,
    to_currency: &str,
) -> StoreResult<i64> {
    if from_currency == to_currency {
        return Ok(amount_cents);
    }

    let rate = latest_rate_tx(conn, from_currency, to_currency)
        .await?
        .ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(format!(
                "No exchange rate available from {} to {}",
                from_currency, to_currency
            )))
        })?;

    let rate_decimal = Decimal::try_from(rate.rate).map_err(|_| {
        Report::new(StoreError::InvalidArgument(format!(
            "Invalid exchange rate value: {}",
            rate.rate
        )))
    })?;

    convert_amount_with_rate(amount_cents, rate_decimal, from_currency, to_currency)
}

/// `rate` is major-unit while `amount_cents` is source minor units, so the product is rescaled
/// by the exponent delta; checked arithmetic, since an overflow must surface as an error.
fn convert_amount_with_rate(
    amount_cents: i64,
    rate: Decimal,
    from_currency: &str,
    to_currency: &str,
) -> StoreResult<i64> {
    let out_of_range = || {
        Report::new(StoreError::InvalidArgument(format!(
            "Converted amount out of range converting {from_currency} -> {to_currency}"
        )))
    };
    let product = Decimal::from(amount_cents)
        .checked_mul(rate)
        .ok_or_else(out_of_range)?;
    let converted = rescale_to_target_minor_units(product, from_currency, to_currency)
        .ok_or_else(out_of_range)?;
    round_to_minor_units(converted).ok_or_else(out_of_range)
}

/// Minor-unit exponent for a currency (digits after the decimal point), defaulting to 2 for
/// currencies without ISO data. Uses the shared precision source so FX rescaling matches how
/// amounts are stored and rendered elsewhere.
fn minor_unit_exponent(currency: &str) -> i32 {
    crate::constants::Currencies::resolve_currency_precision(currency)
        .map(i32::from)
        .unwrap_or(2)
}

/// Shifts `source_minor_units * major_unit_rate` by the exponent delta (EUR 2, JPY 0, KWD 3) into
/// the target currency's minor units.
fn rescale_to_target_minor_units(
    product: Decimal,
    from_currency: &str,
    to_currency: &str,
) -> Option<Decimal> {
    let delta = minor_unit_exponent(to_currency) - minor_unit_exponent(from_currency);
    match delta.cmp(&0) {
        std::cmp::Ordering::Equal => Some(product),
        std::cmp::Ordering::Greater => product.checked_mul(Decimal::from(10i64.pow(delta as u32))),
        std::cmp::Ordering::Less => product.checked_div(Decimal::from(10i64.pow((-delta) as u32))),
    }
}

/// Round a minor-unit amount to a whole integer, half away from zero (matching `ToSubunit`, the
/// rest of the money code). Returns `None` if the value doesn't fit in `i64`.
fn round_to_minor_units(amount: Decimal) -> Option<i64> {
    amount
        .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
        .to_i64()
}

#[cfg(test)]
mod convert_tests {
    use super::{convert_amount_with_rate, rescale_to_target_minor_units, round_to_minor_units};
    use rust_decimal::Decimal;

    #[test]
    fn converts_eur_to_jpy_dropping_two_minor_digits() {
        // €50.00 (5000 cents) at 163 JPY/EUR -> ¥8150 (JPY has 0 minor digits).
        assert_eq!(
            convert_amount_with_rate(5000, Decimal::from(163), "EUR", "JPY").unwrap(),
            8150
        );
    }

    #[test]
    fn converts_jpy_to_eur_adding_two_minor_digits() {
        // ¥8150 at 0.006 EUR/JPY -> €48.90 (4890 cents).
        assert_eq!(
            convert_amount_with_rate(8150, Decimal::new(6, 3), "JPY", "EUR").unwrap(),
            4890
        );
    }

    #[test]
    fn iqd_is_treated_as_two_minor_digits() {
        // Guards the deliberate IQD-at-2 choice through conversion: $10.00 at 1300 IQD/USD ->
        // 1_300_000 IQD minor units. If IQD were ever "corrected" to ISO exp 3 this would be 10x.
        assert_eq!(
            convert_amount_with_rate(1000, Decimal::from(1300), "USD", "IQD").unwrap(),
            1_300_000
        );
    }

    #[test]
    fn out_of_range_product_errors_instead_of_panicking() {
        // Huge amount * huge rate overflows Decimal's range: must be an error, never a panic or 0.
        assert!(convert_amount_with_rate(i64::MAX, Decimal::from(i64::MAX), "USD", "JPY").is_err());
    }

    #[test]
    fn rounds_half_away_from_zero() {
        // 81.5 -> 82 (away from zero), not 80 (banker's). Cross-exponent rescales produce
        // fractional minor units routinely, so this tie must match the rest of the money code.
        assert_eq!(round_to_minor_units(Decimal::new(815, 1)), Some(82));
        assert_eq!(round_to_minor_units(Decimal::new(-815, 1)), Some(-82));
        assert_eq!(round_to_minor_units(Decimal::new(825, 1)), Some(83));
    }

    #[test]
    fn out_of_range_amount_is_none_not_zero() {
        // Beyond i64: must not silently collapse to 0 (caller turns None into an error).
        let huge = Decimal::MAX;
        assert_eq!(round_to_minor_units(huge), None);
    }

    #[test]
    fn same_exponent_is_identity() {
        // EUR(2) -> USD(2): 50 cents * 1.08 = 54, already in USD cents.
        assert_eq!(
            rescale_to_target_minor_units(Decimal::from(54), "EUR", "USD"),
            Some(Decimal::from(54))
        );
    }

    #[test]
    fn to_fewer_minor_digits_divides() {
        // EUR(2) -> JPY(0): 50 cents * 163 = 8150 must become ¥81.5 (the raw product is 100x off).
        assert_eq!(
            rescale_to_target_minor_units(Decimal::from(8150), "EUR", "JPY"),
            Some(Decimal::new(815, 1))
        );
    }

    #[test]
    fn to_more_minor_digits_multiplies() {
        // EUR(2) -> BHD(3): one extra minor-unit digit, so scale up by 10.
        assert_eq!(
            rescale_to_target_minor_units(Decimal::from(21), "EUR", "BHD"),
            Some(Decimal::from(210))
        );
    }

    #[test]
    fn from_fewer_minor_digits_multiplies() {
        // JPY(0) -> EUR(2): the product is in whole yen, so scale up by 100 to reach EUR cents.
        assert_eq!(
            rescale_to_target_minor_units(Decimal::from(5), "JPY", "EUR"),
            Some(Decimal::from(500))
        );
    }

    #[test]
    fn unknown_currency_falls_back_to_exponent_two() {
        // An unrecognized code is treated as exponent 2, matching EUR -> identity here.
        assert_eq!(
            rescale_to_target_minor_units(Decimal::from(42), "EUR", "XZZ"),
            Some(Decimal::from(42))
        );
    }
}
