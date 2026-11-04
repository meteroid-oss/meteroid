use crate::domain::stats::{
    CountAndValue, SignupDataPoint, SignupSeries, SignupSparklineResponse, Trend, TrendScope,
    TrialConversionDataPoint, TrialConversionMetaDataPoint, TrialConversionRateResponse,
    TrialConversionSeries,
};
use crate::errors::StoreError;
use crate::repositories::TenantInterface;
use crate::repositories::historical_rates::convert_currency;
use crate::{Store, StoreResult};
use common_domain::ids::TenantId;
use diesel_models::stats::{
    ActiveSubscriptionsCountRow, DailyNewSignups90DaysRow, NewSignupsTrend90DaysRow,
    PendingInvoicesByCurrencyRow, SubscriptionTrialConversionRateRow,
    SubscriptionTrialToPaidConversionRow,
};
use error_stack::Report;
use rust_decimal::prelude::ToPrimitive;

/// Operational dashboard counters over live Postgres tables. The financial
/// stats (MRR, revenue, movement log, top customers) moved to the analytics
/// plane — see `UsageClient::query_mrr_timeseries` & friends.
#[async_trait::async_trait]
pub trait StatsInterface {
    async fn active_subscriptions(&self, tenant_id: TenantId) -> StoreResult<i64>;
    async fn pending_invoices(&self, tenant_id: TenantId) -> StoreResult<CountAndValue>;
    async fn signups(&self, tenant_id: TenantId) -> StoreResult<Trend>;
    async fn signups_sparkline(&self, tenant_id: TenantId) -> StoreResult<SignupSparklineResponse>;
    async fn trial_conversion_rate(&self, tenant_id: TenantId) -> StoreResult<f32>;
    async fn trial_conversion_rate_sparkline(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<TrialConversionRateResponse>;
}

#[async_trait::async_trait]
impl StatsInterface for Store {
    async fn active_subscriptions(&self, tenant_id: TenantId) -> StoreResult<i64> {
        let mut conn = self.get_conn().await?;

        ActiveSubscriptionsCountRow::get(&mut conn, tenant_id)
            .await
            .map_err(Into::into)
            .map(|x| x.count.into())
    }

    async fn pending_invoices(&self, tenant_id: TenantId) -> StoreResult<CountAndValue> {
        let mut conn = self.get_conn().await?;

        let by_currency = PendingInvoicesByCurrencyRow::list(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let reporting_currency = self.get_reporting_currency_by_tenant_id(tenant_id).await?;

        // `total` is already in the invoice currency's minor units; convert each to the reporting
        // currency's minor units so the fold needs no further rescaling. A currency with no usable
        // FX rate yields `None` — counted but excluded from `value` (see `fold_pending_totals`).
        let mut converted = Vec::with_capacity(by_currency.len());
        for row in by_currency {
            let value = match convert_currency(
                &mut conn,
                row.total,
                &row.currency,
                reporting_currency.code,
            )
            .await
            {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(
                        currency = %row.currency,
                        error = %e,
                        "skipping pending-invoice total with no usable FX rate"
                    );
                    None
                }
            };
            converted.push((row.count, value));
        }

        Ok(fold_pending_totals(converted))
    }

    async fn signups(&self, tenant_id: TenantId) -> StoreResult<Trend> {
        let mut conn = self.get_conn().await?;

        let trend = NewSignupsTrend90DaysRow::get(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let (change, percent) =
            calculate_trend(trend.total_last_90_days, trend.total_previous_90_days);

        Ok(Trend {
            current: trend.total_last_90_days,
            change_amount: change,
            change_percent: percent,
            positive_is_good: true,
            scope: TrendScope::Trend30d,
        })
    }

    async fn signups_sparkline(&self, tenant_id: TenantId) -> StoreResult<SignupSparklineResponse> {
        let mut conn = self.get_conn().await?;

        let chart_data = DailyNewSignups90DaysRow::list(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let series = SignupSeries {
            name: "New signups".to_string(),
            code: "new_signups".to_string(),
            data: chart_data
                .iter()
                .map(|d| SignupDataPoint {
                    x: d.signup_date.to_string(),
                    total: d.total_signups_over_30_days,
                    delta: d.daily_signups,
                })
                .collect(),
        };

        Ok(SignupSparklineResponse { series })
    }

    async fn trial_conversion_rate(&self, tenant_id: TenantId) -> StoreResult<f32> {
        let mut conn = self.get_conn().await?;

        let all_time = SubscriptionTrialConversionRateRow::get(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(all_time
            .all_time_conversion_rate_percentage
            .round_dp(1)
            .to_f32()
            .unwrap_or(0.0))
    }

    async fn trial_conversion_rate_sparkline(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<TrialConversionRateResponse> {
        let mut conn = self.get_conn().await?;

        let chart_data = SubscriptionTrialToPaidConversionRow::list(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut conversions_series = TrialConversionSeries {
            name: "Trial conversion rate".into(),
            code: "trial_conversion_rate".into(),
            data: Vec::new(),
        };
        let mut conversions_30_days_series = TrialConversionSeries {
            name: "Trial-to-paid under 30 Days rate".into(),
            code: "trial_conversion_rate_30_days".into(),
            data: Vec::new(),
        };
        let mut conversions_90_days_series = TrialConversionSeries {
            name: "Trial-to-paid under 90 Days rate".into(),
            code: "trial_conversion_rate_90_days".into(),
            data: Vec::new(),
        };

        let mut metadata_series = Vec::new();

        for dp in chart_data {
            let month_str = dp.month.format("%Y-%m").to_string();

            metadata_series.push(TrialConversionMetaDataPoint {
                x: month_str.clone(),
                total_trials: dp.total_trials,
            });
            conversions_series.data.push(TrialConversionDataPoint {
                x: month_str.clone(),
                conversion_rate: dp
                    .conversion_rate_percentage
                    .round_dp(1)
                    .to_f32()
                    .unwrap_or(0.0),
                conversions: dp.conversions,
            });
            conversions_30_days_series
                .data
                .push(TrialConversionDataPoint {
                    x: month_str.clone(),
                    conversion_rate: dp
                        .conversion_rate_30_percentage
                        .round_dp(1)
                        .to_f32()
                        .unwrap_or(0.0),
                    conversions: dp.conversions_30,
                });
            conversions_90_days_series
                .data
                .push(TrialConversionDataPoint {
                    x: month_str,
                    conversion_rate: dp
                        .conversion_rate_90_percentage
                        .round_dp(1)
                        .to_f32()
                        .unwrap_or(0.0),
                    conversions: dp.conversions_90,
                });
        }

        let series = vec![
            conversions_series,
            conversions_30_days_series,
            conversions_90_days_series,
        ];

        Ok(TrialConversionRateResponse {
            series,
            metadata: metadata_series,
        })
    }
}

fn calculate_trend(current: i64, previous: i64) -> (i64, f32) {
    let change = current - previous;
    let change_percent = if previous == 0 {
        0f64
    } else {
        (change as f64 / previous as f64) * 100.0
    };
    let change_percent_rounded = (change_percent * 10.0).round() / 10.0;
    (change, change_percent_rounded as f32)
}

/// Folds per-currency pending rows into one count/value. Every row's `count` is included, but a
/// row whose total could not be converted (`None`, no FX rate) adds nothing to `value` — matching
/// the old SQL where a NULL rate dropped out of SUM yet the invoice was still counted.
fn fold_pending_totals(rows: impl IntoIterator<Item = (i64, Option<i64>)>) -> CountAndValue {
    let (count, value) = rows
        .into_iter()
        .fold((0i64, 0i64), |(c, v), (count, value)| {
            (c + count, v + value.unwrap_or(0))
        });
    CountAndValue {
        count: count.try_into().unwrap_or(i32::MAX),
        value,
    }
}

#[cfg(test)]
mod tests {
    use super::fold_pending_totals;

    #[test]
    fn counts_every_row_but_sums_value_only_for_convertible_ones() {
        // Two EUR invoices (converted), one JPY (converted), four in a currency with no FX rate:
        // all seven are counted, but the unconvertible total is excluded from `value`.
        let result = fold_pending_totals([(2, Some(12000)), (1, Some(3000)), (4, None)]);
        assert_eq!(result.count, 7);
        assert_eq!(result.value, 15000);
    }

    #[test]
    fn empty_is_zero() {
        let result = fold_pending_totals(std::iter::empty::<(i64, Option<i64>)>());
        assert_eq!(result.count, 0);
        assert_eq!(result.value, 0);
    }
}
