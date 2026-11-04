use chrono::NaiveDate;
use common_domain::ids::{CustomerId, PlanId};
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
    pub revenue_ytd: i64,
    pub revenue_all_time: i64,
    pub currency: String,
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
    pub mrr_change: i64,
    pub currency: String,
}

pub struct RevenueChartDataPoint {
    pub x: String,
    pub revenue: i64,
    pub daily_revenue: i64,
}

/// Opening MRR balance (sum of movement deltas strictly before a range), per
/// plan when the source query split by plan. Read from ClickHouse
/// `fact_mrr_movement`, already in the tenant reporting currency.
pub struct MrrOpeningBalanceRow {
    pub plan_id: Option<PlanId>,
    pub mrr_cents: i64,
}

/// One aggregated day of MRR movements in the reporting currency, as read from
/// ClickHouse. Sparse — days without movements are absent.
#[derive(Default, Clone)]
pub struct MrrTimeseriesPointRow {
    pub date: NaiveDate,
    pub plan_id: Option<PlanId>,
    pub plan_name: Option<String>,
    pub net_mrr_cents: i64,
    pub new_business_cents: i64,
    pub new_business_count: i32,
    pub expansion_cents: i64,
    pub expansion_count: i32,
    pub contraction_cents: i64,
    pub contraction_count: i32,
    pub churn_cents: i64,
    pub churn_count: i32,
    pub reactivation_cents: i64,
    pub reactivation_count: i32,
}

pub struct MrrTimeseriesRows {
    pub opening: Vec<MrrOpeningBalanceRow>,
    pub points: Vec<MrrTimeseriesPointRow>,
}

impl MrrTimeseriesRows {
    /// Total MRR at the end of the queried range: opening balance plus every
    /// in-range movement (all plans combined).
    pub fn total_mrr_cents(&self) -> i64 {
        let opening: i64 = self.opening.iter().map(|o| o.mrr_cents).sum();
        opening + self.points.iter().map(|p| p.net_mrr_cents).sum::<i64>()
    }
}

/// One day of billed net revenue in the reporting currency, as read from
/// ClickHouse. Sparse.
pub struct RevenueDailyPointRow {
    pub date: NaiveDate,
    pub net_revenue_cents: i64,
}

fn breakdown_from_point(p: &MrrTimeseriesPointRow, total_net_mrr: i64) -> MRRBreakdown {
    MRRBreakdown {
        new_business: CountAndValue {
            count: p.new_business_count,
            value: p.new_business_cents,
        },
        expansion: CountAndValue {
            count: p.expansion_count,
            value: p.expansion_cents,
        },
        contraction: CountAndValue {
            count: p.contraction_count,
            value: p.contraction_cents,
        },
        churn: CountAndValue {
            count: p.churn_count,
            value: p.churn_cents,
        },
        reactivation: CountAndValue {
            count: p.reactivation_count,
            value: p.reactivation_cents,
        },
        net_new_mrr: p.net_mrr_cents,
        total_net_mrr,
    }
}

/// Fill `[from, to]` day by day and accumulate `opening + net` into a running
/// total, producing the daily points of one MRR chart series.
fn accumulate_mrr_series(
    opening_cents: i64,
    points: &[&MrrTimeseriesPointRow],
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<MrrChartDataPoint> {
    let by_date: std::collections::HashMap<NaiveDate, &MrrTimeseriesPointRow> =
        points.iter().map(|p| (p.date, *p)).collect();
    let empty = MrrTimeseriesPointRow::default();
    let mut running = opening_cents;
    let mut out = Vec::new();
    let mut day = from;
    while day <= to {
        let p = by_date.get(&day).copied().unwrap_or(&empty);
        running += p.net_mrr_cents;
        out.push(MrrChartDataPoint {
            x: day.format("%Y-%m-%d").to_string(),
            data: breakdown_from_point(p, running),
        });
        day = day.succ_opt().expect("date range within calendar bounds");
    }
    out
}

/// The single-series ("Total MRR") chart over `[from, to]` from ClickHouse
/// movement rows: gaps filled, running total accumulated from the opening balance.
pub fn build_total_mrr_chart(
    rows: &MrrTimeseriesRows,
    from: NaiveDate,
    to: NaiveDate,
) -> MrrChartResponse {
    let opening: i64 = rows.opening.iter().map(|o| o.mrr_cents).sum();
    let points: Vec<&MrrTimeseriesPointRow> = rows.points.iter().collect();
    MrrChartResponse {
        series: vec![MrrChartSeries {
            name: "Total MRR".to_string(),
            code: "total_mrr".to_string(),
            plan: None,
            data: accumulate_mrr_series(opening, &points, from, to),
        }],
    }
}

/// One chart series per selected plan over `[from, to]`. Every requested plan
/// gets a full series (flat at its opening balance when it had no movements),
/// matching the legacy per-plan chart. `plans` supplies the labels.
pub fn build_mrr_chart_by_plan(
    rows: &MrrTimeseriesRows,
    plans: &[PlanBrief],
    from: NaiveDate,
    to: NaiveDate,
) -> MrrChartResponse {
    let series = plans
        .iter()
        .map(|plan| {
            let opening = rows
                .opening
                .iter()
                .filter(|o| o.plan_id == Some(plan.id))
                .map(|o| o.mrr_cents)
                .sum();
            let points: Vec<&MrrTimeseriesPointRow> = rows
                .points
                .iter()
                .filter(|p| p.plan_id == Some(plan.id))
                .collect();
            MrrChartSeries {
                name: plan.name.clone(),
                code: format!("mrr_breakdown_plan_{}", plan.id),
                plan: Some(PlanBrief {
                    id: plan.id,
                    name: plan.name.clone(),
                }),
                data: accumulate_mrr_series(opening, &points, from, to),
            }
        })
        .collect();
    MrrChartResponse { series }
}

/// Movement totals over a range (the MRR breakdown card): counts and amounts
/// summed across days. `total_net_mrr` stays 0, matching the legacy endpoint.
pub fn sum_mrr_breakdown(points: &[MrrTimeseriesPointRow]) -> MRRBreakdown {
    let mut acc = MrrTimeseriesPointRow::default();
    for p in points {
        acc.net_mrr_cents += p.net_mrr_cents;
        acc.new_business_cents += p.new_business_cents;
        acc.new_business_count += p.new_business_count;
        acc.expansion_cents += p.expansion_cents;
        acc.expansion_count += p.expansion_count;
        acc.contraction_cents += p.contraction_cents;
        acc.contraction_count += p.contraction_count;
        acc.churn_cents += p.churn_cents;
        acc.churn_count += p.churn_count;
        acc.reactivation_cents += p.reactivation_cents;
        acc.reactivation_count += p.reactivation_count;
    }
    breakdown_from_point(&acc, 0)
}

/// The revenue chart over `[from, to]` from sparse daily rows: gaps filled,
/// cumulative revenue accumulated from zero within the range (legacy semantics).
pub fn build_revenue_chart(
    points: &[RevenueDailyPointRow],
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<RevenueChartDataPoint> {
    let by_date: std::collections::HashMap<NaiveDate, i64> = points
        .iter()
        .map(|p| (p.date, p.net_revenue_cents))
        .collect();
    let mut running = 0i64;
    let mut out = Vec::new();
    let mut day = from;
    while day <= to {
        let daily = by_date.get(&day).copied().unwrap_or(0);
        running += daily;
        out.push(RevenueChartDataPoint {
            x: day.format("%Y-%m-%d").to_string(),
            revenue: running,
            daily_revenue: daily,
        });
        day = day.succ_opt().expect("date range within calendar bounds");
    }
    out
}

/// The dashboard net-revenue trend from daily rows covering at least
/// `[min(start_of_year, today − 2×period), today]`: year-to-date total plus the
/// trailing `period_days` window compared to the window before it.
pub fn revenue_trend(points: &[RevenueDailyPointRow], today: NaiveDate, period_days: u32) -> Trend {
    let year_start = crate::utils::datetime::start_of_year(today);
    let current_start = today - chrono::Days::new(period_days as u64);
    let previous_start = today - chrono::Days::new(2 * period_days as u64);

    let mut total_ytd = 0i64;
    let mut current = 0i64;
    let mut previous = 0i64;
    for p in points {
        if p.date >= year_start && p.date <= today {
            total_ytd += p.net_revenue_cents;
        }
        if p.date > current_start && p.date <= today {
            current += p.net_revenue_cents;
        } else if p.date > previous_start && p.date <= current_start {
            previous += p.net_revenue_cents;
        }
    }

    let change = current - previous;
    let change_percent = if previous == 0 {
        0.0
    } else {
        ((change as f64 / previous as f64) * 1000.0).round() / 10.0
    };
    Trend {
        current: total_ytd,
        change_amount: change,
        change_percent: change_percent as f32,
        positive_is_good: true,
        scope: TrendScope::Trend7d,
    }
}

#[cfg(test)]
mod dashboard_shaping_tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn point(date: &str, net: i64) -> MrrTimeseriesPointRow {
        MrrTimeseriesPointRow {
            date: d(date),
            net_mrr_cents: net,
            ..Default::default()
        }
    }

    #[test]
    fn total_mrr_chart_fills_gaps_and_accumulates_from_opening() {
        let rows = MrrTimeseriesRows {
            opening: vec![MrrOpeningBalanceRow {
                plan_id: None,
                mrr_cents: 1000,
            }],
            points: vec![point("2026-01-02", 500), point("2026-01-04", -200)],
        };
        let chart = build_total_mrr_chart(&rows, d("2026-01-01"), d("2026-01-04"));
        let data = &chart.series[0].data;
        assert_eq!(data.len(), 4, "every day of the range is present");
        let totals: Vec<i64> = data.iter().map(|p| p.data.total_net_mrr).collect();
        assert_eq!(totals, vec![1000, 1500, 1500, 1300]);
        assert_eq!(data[1].data.net_new_mrr, 500);
        assert_eq!(data[2].data.net_new_mrr, 0, "gap day is zero-movement");
        assert_eq!(rows.total_mrr_cents(), 1300);
    }

    #[test]
    fn per_plan_chart_keeps_movement_free_plans_flat_at_opening() {
        use common_domain::ids::BaseId;
        let plan_a = PlanId::new();
        let plan_b = PlanId::new();
        let rows = MrrTimeseriesRows {
            opening: vec![MrrOpeningBalanceRow {
                plan_id: Some(plan_a),
                mrr_cents: 700,
            }],
            points: vec![MrrTimeseriesPointRow {
                plan_id: Some(plan_a),
                ..point("2026-01-02", 300)
            }],
        };
        let plans = vec![
            PlanBrief {
                id: plan_a,
                name: "Pro".into(),
            },
            PlanBrief {
                id: plan_b,
                name: "Free".into(),
            },
        ];
        let chart = build_mrr_chart_by_plan(&rows, &plans, d("2026-01-01"), d("2026-01-02"));
        assert_eq!(chart.series.len(), 2, "every selected plan gets a series");
        let pro = chart.series.iter().find(|s| s.name == "Pro").unwrap();
        assert_eq!(
            pro.data.last().unwrap().data.total_net_mrr,
            1000,
            "opening + movement"
        );
        let free = chart.series.iter().find(|s| s.name == "Free").unwrap();
        assert!(
            free.data.iter().all(|p| p.data.total_net_mrr == 0),
            "no movements, no opening — flat zero"
        );
    }

    #[test]
    fn breakdown_sums_counts_and_amounts_across_days() {
        let mut p1 = point("2026-01-01", 800);
        p1.new_business_cents = 1000;
        p1.new_business_count = 2;
        p1.churn_cents = -200;
        p1.churn_count = 1;
        let mut p2 = point("2026-01-02", 300);
        p2.new_business_cents = 300;
        p2.new_business_count = 1;
        let b = sum_mrr_breakdown(&[p1, p2]);
        assert_eq!(b.new_business.count, 3);
        assert_eq!(b.new_business.value, 1300);
        assert_eq!(b.churn.count, 1);
        assert_eq!(b.churn.value, -200);
        assert_eq!(b.net_new_mrr, 1100);
        assert_eq!(b.total_net_mrr, 0, "legacy endpoint reports 0 here");
    }

    #[test]
    fn revenue_chart_accumulates_from_zero_within_range() {
        let points = vec![
            RevenueDailyPointRow {
                date: d("2026-01-02"),
                net_revenue_cents: 100,
            },
            RevenueDailyPointRow {
                date: d("2026-01-03"),
                net_revenue_cents: 50,
            },
        ];
        let chart = build_revenue_chart(&points, d("2026-01-01"), d("2026-01-03"));
        let cum: Vec<i64> = chart.iter().map(|p| p.revenue).collect();
        assert_eq!(cum, vec![0, 100, 150]);
        assert_eq!(chart[1].daily_revenue, 100);
    }

    #[test]
    fn revenue_trend_compares_trailing_windows_and_totals_ytd() {
        let today = d("2026-03-10");
        let points = vec![
            // YTD but outside both trailing windows.
            RevenueDailyPointRow {
                date: d("2026-01-15"),
                net_revenue_cents: 1000,
            },
            // Previous 7-day window (03-03 >= date > 02-24).
            RevenueDailyPointRow {
                date: d("2026-03-01"),
                net_revenue_cents: 200,
            },
            // Current 7-day window (03-10 >= date > 03-03).
            RevenueDailyPointRow {
                date: d("2026-03-08"),
                net_revenue_cents: 500,
            },
        ];
        let t = revenue_trend(&points, today, 7);
        assert_eq!(t.current, 1700, "YTD total");
        assert_eq!(t.change_amount, 300, "current vs previous window");
        assert_eq!(t.change_percent, 150.0);
    }
}
