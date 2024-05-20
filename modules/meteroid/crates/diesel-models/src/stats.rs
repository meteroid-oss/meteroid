use crate::enums::MrrMovementType;
use chrono::{NaiveDate, NaiveDateTime};
use diesel::QueryableByName;
use rust_decimal::Decimal;
use uuid::Uuid;

#[derive(QueryableByName, Debug)]
pub struct RevenueTrend {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_ytd: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_current_period: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_previous_period: i64,
}

#[derive(QueryableByName, Debug)]
pub struct NewSignupsTrend90Days {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_last_90_days: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_previous_90_days: i64,
}

pub struct ActiveSubscriptionsCount {
    pub count: i32,
}

#[derive(QueryableByName, Debug)]
pub struct PendingInvoicesTotal {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub total: i32,
    #[diesel(sql_type = diesel::sql_types::Decimal)]
    pub total_cents: Decimal,
}

#[derive(QueryableByName, Debug)]
pub struct DailyNewSignups90Days {
    #[diesel(sql_type = diesel::sql_types::Date)]
    pub signup_date: NaiveDate,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub daily_signups: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_signups_over_30_days: i64,
}

#[derive(QueryableByName, Debug)]
pub struct SubscriptionTrialConversionRate {
    #[diesel(sql_type = diesel::sql_types::Decimal)]
    pub all_time_conversion_rate_percentage: Decimal,
}

#[derive(QueryableByName, Debug)]
pub struct SubscriptionTrialToPaidConversion {
    #[diesel(sql_type = diesel::sql_types::Date)]
    pub month: NaiveDate,
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

#[derive(QueryableByName, Debug)]
pub struct CustomerTopRevenue {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub name: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_revenue_cents: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub currency: String,
}

#[derive(QueryableByName, Debug)]
pub struct TotalMrr {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_net_mrr_cents: i64,
}

#[derive(QueryableByName, Debug)]
pub struct TotalMrrChart {
    #[diesel(sql_type = diesel::sql_types::Date)]
    pub period: NaiveDate,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_net_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub net_new_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub new_business_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub new_business_count: i32,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub expansion_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub expansion_count: i32,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub contraction_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub contraction_count: i32,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub churn_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub churn_count: i32,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub reactivation_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub reactivation_count: i32,
}

#[derive(QueryableByName, Debug)]
pub struct MrrBreakdown {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub net_new_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub new_business_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub new_business_count: i32,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub expansion_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub expansion_count: i32,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub contraction_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub contraction_count: i32,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub churn_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub churn_count: i32,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub reactivation_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub reactivation_count: i32,
}

#[derive(QueryableByName, Debug)]
pub struct LastMrrMovements {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: Uuid,
    #[diesel(sql_type = crate::schema::sql_types::MrrMovementType)]
    pub movement_type: MrrMovementType,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub net_mrr_change: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub currency: String,
    #[diesel(sql_type = diesel::sql_types::Date)]
    pub applies_to: NaiveDate,
    #[diesel(sql_type = diesel::sql_types::Timestamp)]
    pub created_at: NaiveDateTime,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub description: String,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub invoice_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
    pub credit_note_id: Option<Uuid>,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub tenant_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub plan_version_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub customer_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub customer_name: String,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub subscription_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub plan_name: String,
}

#[derive(QueryableByName, Debug)]
pub struct TotalMrrByPlan {
    #[diesel(sql_type = diesel::sql_types::Date)]
    pub date: NaiveDate,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub plan_id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub plan_name: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_net_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub net_new_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub new_business_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub new_business_count: i32,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub expansion_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub expansion_count: i32,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub contraction_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub contraction_count: i32,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub churn_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub churn_count: i32,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub reactivation_mrr: i64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub reactivation_count: i32,
}
