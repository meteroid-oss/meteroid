use chrono::NaiveDate;
use common_domain::ids::{CustomerId, PlanId, TenantId};
pub enum TrendScope {
    Trend24h = 0,
    Trend7d = 1,
    Trend30d = 2,
    Trend90d = 3,
    Trend1y = 4,
    Trend2y = 5,
}

pub struct Trend {
    pub current: i64,
    pub change_amount: i64,
    pub change_percent: f32,
    pub positive_is_good: bool,
    pub scope: TrendScope,
}

pub struct SignupDataPoint {
    pub x: String,
    pub total: i64,
    pub delta: i64,
}

pub struct SignupSeries {
    pub name: String,
    pub code: String,
    pub data: Vec<SignupDataPoint>,
}

pub struct SignupSparklineResponse {
    pub series: SignupSeries,
}

pub struct TrialConversionRateResponse {
    pub series: Vec<TrialConversionSeries>,
    pub metadata: Vec<TrialConversionMetaDataPoint>,
}

pub struct TrialConversionMetaDataPoint {
    pub x: String,
    pub total_trials: i64,
}

pub struct TrialConversionSeries {
    pub name: String,
    pub code: String,
    pub data: Vec<TrialConversionDataPoint>,
}

pub struct TrialConversionDataPoint {
    pub x: String,
    pub conversion_rate: f32,
    pub conversions: i64,
}

pub struct RevenueByCustomer {
    pub customer_name: String,
    pub customer_id: CustomerId,
    pub revenue: i64,
    pub currency: String,
}

pub struct RevenueByCustomerRequest {
    pub limit: u32,
    pub tenant_id: TenantId,
    pub currency: Option<String>,
}

pub struct MrrChartRequest {
    pub tenant_id: TenantId,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub plans_id: Option<Vec<PlanId>>,
}

pub struct MrrChartResponse {
    pub series: Vec<MrrChartSeries>,
}

pub struct MrrChartDataPoint {
    pub x: String,
    pub data: MRRBreakdown,
}

pub struct MrrChartSeries {
    pub name: String,
    pub code: String,
    pub plan: Option<PlanBrief>,
    pub data: Vec<MrrChartDataPoint>,
}

pub struct PlanBrief {
    pub id: PlanId,
    pub name: String,
}

pub enum MRRBreakdownScope {
    ThisWeek,
    ThisMonth,
    ThisQuarter,
    ThisYear,
    LastWeek,
    LastMonth,
    LastQuarter,
    LastYear,
}

impl MRRBreakdownScope {
    pub fn to_date_range(&self, now: NaiveDate) -> (NaiveDate, NaiveDate) {
        use crate::utils::datetime::{
            end_of_month, end_of_quarter, end_of_week, end_of_year, start_of_month,
            start_of_quarter, start_of_week, start_of_year, sub_months,
        };

        match self {
            MRRBreakdownScope::ThisWeek => (start_of_week(now), now),
            MRRBreakdownScope::ThisMonth => (start_of_month(now), now),
            MRRBreakdownScope::ThisQuarter => (start_of_quarter(now), now),
            MRRBreakdownScope::ThisYear => (start_of_year(now), now),
            MRRBreakdownScope::LastWeek => (
                start_of_week(now)
                    .checked_sub_days(chrono::Days::new(7))
                    .unwrap(),
                end_of_week(now)
                    .checked_sub_days(chrono::Days::new(7))
                    .unwrap(),
            ),
            MRRBreakdownScope::LastMonth => (
                start_of_month(sub_months(now, 1)),
                end_of_month(sub_months(now, 1)),
            ),
            MRRBreakdownScope::LastQuarter => (
                start_of_quarter(sub_months(now, 3)),
                end_of_quarter(sub_months(now, 3)),
            ),
            MRRBreakdownScope::LastYear => (
                start_of_year(sub_months(now, 12)),
                end_of_year(sub_months(now, 12)),
            ),
        }
    }
}

pub struct MRRBreakdownRequest {
    pub tenant_id: TenantId,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
}

pub struct CountAndValue {
    pub count: i32,
    pub value: i64,
}

pub struct MRRBreakdown {
    pub new_business: CountAndValue,
    pub expansion: CountAndValue,
    pub contraction: CountAndValue,
    pub churn: CountAndValue,
    pub reactivation: CountAndValue,
    pub net_new_mrr: i64,
    pub total_net_mrr: i64,
    // scheduled_mrr_movements: CountAndValue,
}

pub struct MrrLogRequest {
    pub tenant_id: TenantId,
    pub before: Option<String>,
    pub after: Option<String>,
}

pub struct MrrLogResponse {
    pub cursor: String,
    pub entries: Vec<MrrLogEntry>,
}

pub enum MrrMovementType {
    NewBusiness,
    Expansion,
    Contraction,
    Churn,
    Reactivation,
}

pub struct MrrLogEntry {
    pub created_at: chrono::NaiveDateTime,
    pub applies_to: NaiveDate,
    pub customer_id: String,
    pub customer_name: String,
    pub subscription_id: String,
    pub plan_name: String,
    pub description: String,
    pub mrr_type: MrrMovementType,
}

pub struct RevenueChartRequest {
    pub tenant_id: TenantId,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub plans_id: Option<Vec<PlanId>>,
}

pub struct RevenueChartResponse {
    pub series: Vec<RevenueChartSeries>,
}

pub struct RevenueChartSeries {
    pub name: String,
    pub code: String,
    pub data: Vec<RevenueChartDataPoint>,
}

pub struct RevenueChartDataPoint {
    pub x: String,
    pub revenue: i64,
}
