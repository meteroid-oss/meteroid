use chrono::{NaiveDate, NaiveDateTime};
use diesel::QueryableByName;
use rust_decimal::Decimal;

#[derive(QueryableByName, Debug)]
pub struct NewSignupsTrend90DaysRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_last_90_days: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_previous_90_days: i64,
}

pub struct ActiveSubscriptionsCountRow {
    pub count: i32,
}

/// Pending (finalized, unpaid) invoice counts and totals grouped by the invoice's own
/// currency. Cross-currency conversion to the tenant's reporting currency happens in Rust so
/// it shares the single FX path (`convert_currency`) and its minor-unit exponent handling.
#[derive(QueryableByName, Debug)]
pub struct PendingInvoicesByCurrencyRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub currency: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub count: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total: i64,
}

#[derive(QueryableByName, Debug)]
pub struct DailyNewSignups90DaysRow {
    #[diesel(sql_type = diesel::sql_types::Date)]
    pub signup_date: NaiveDate,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub daily_signups: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_signups_over_30_days: i64,
}

#[derive(QueryableByName, Debug)]
pub struct SubscriptionTrialConversionRateRow {
    #[diesel(sql_type = diesel::sql_types::Decimal)]
    pub all_time_conversion_rate_percentage: Decimal,
}

#[derive(QueryableByName, Debug)]
pub struct SubscriptionTrialToPaidConversionRow {
    #[diesel(sql_type = diesel::sql_types::Timestamp)]
    pub month: NaiveDateTime,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_trials: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub conversions: i64,
    #[diesel(sql_type = diesel::sql_types::Decimal)]
    pub conversion_rate_percentage: Decimal,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub conversions_30: i64,
    #[diesel(sql_type = diesel::sql_types::Decimal)]
    pub conversion_rate_30_percentage: Decimal,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub conversions_90: i64,
    #[diesel(sql_type = diesel::sql_types::Decimal)]
    pub conversion_rate_90_percentage: Decimal,
}
