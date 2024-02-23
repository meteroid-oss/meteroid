use crate::compute::fees::shared::ToCents;

use cornucopia_async::Params;
use deadpool_postgres::{Object, Pool};

use meteroid_repository as db;
use meteroid_repository::stats::{
    GetMrrBreakdownParams, QueryTotalMrrByPlanParams, QueryTotalMrrParams,
};
use rust_decimal::prelude::ToPrimitive;
use std::collections::HashMap;
use thiserror::Error;

use crate::services::stats::utils::date_utils;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum StatServiceError {
    #[error("Stat not found")]
    NotFound,
    #[error("Failed to query stat : {0}")]
    InternalServerError(String),
}

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
    pub customer_id: Uuid,
    pub revenue: i64,
}

pub struct RevenueByCustomerRequest {
    pub limit: u32,
    pub tenant_id: Uuid,
}

pub struct MrrChartRequest {
    pub tenant_id: Uuid,
    pub start_date: time::Date,
    pub end_date: time::Date,
    pub plans_id: Option<Vec<Uuid>>,
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
    pub plan: Option<Plan>,
    pub data: Vec<MrrChartDataPoint>,
}
pub struct Plan {
    pub id: Uuid,
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
    fn to_date_range(&self, now: time::Date) -> (time::Date, time::Date) {
        use date_utils::*;

        match self {
            MRRBreakdownScope::ThisWeek => (start_of_week(now), now),
            MRRBreakdownScope::ThisMonth => (start_of_month(now), now),
            MRRBreakdownScope::ThisQuarter => (start_of_quarter(now), now),
            MRRBreakdownScope::ThisYear => (start_of_year(now), now),
            MRRBreakdownScope::LastWeek => (
                start_of_week(now)
                    .checked_sub(time::Duration::days(7))
                    .unwrap(),
                end_of_week(now)
                    .checked_sub(time::Duration::days(7))
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
    pub scope: MRRBreakdownScope,
    pub tenant_id: Uuid,
}

pub struct CountAndValue {
    pub count: i64,
    pub value: i64,
}
pub struct MRRBreakdown {
    pub new_business: CountAndValue,
    pub expansion: CountAndValue,
    pub contraction: CountAndValue,
    pub churn: CountAndValue,
    pub reactivation: CountAndValue,
    pub net_new_mrr: i64,
    // scheduled_mrr_movements: CountAndValue,
}

pub struct MrrLogRequest {
    pub tenant_id: Uuid,
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
    pub created_at: time::OffsetDateTime,
    pub applies_to: time::Date,
    pub customer_id: String,
    pub customer_name: String,
    pub subscription_id: String,
    pub plan_name: String,
    pub description: String,
    pub mrr_type: MrrMovementType,
}

#[async_trait::async_trait]
pub trait StatsService {
    async fn net_revenue(&self, tenant_id: Uuid) -> Result<Trend, StatServiceError>;
    async fn active_subscriptions(&self, tenant_id: Uuid) -> Result<i64, StatServiceError>;
    async fn pending_invoices(&self, tenant_id: Uuid) -> Result<CountAndValue, StatServiceError>;
    async fn signups(&self, tenant_id: Uuid) -> Result<Trend, StatServiceError>;
    async fn signups_sparkline(
        &self,
        tenant_id: Uuid,
    ) -> Result<SignupSparklineResponse, StatServiceError>;
    async fn trial_conversion_rate(&self, tenant_id: Uuid) -> Result<f32, StatServiceError>;
    async fn trial_conversion_rate_sparkline(
        &self,
        tenant_id: Uuid,
    ) -> Result<TrialConversionRateResponse, StatServiceError>;
    async fn top_revenue_by_customer(
        &self,
        request: RevenueByCustomerRequest,
    ) -> Result<Vec<RevenueByCustomer>, StatServiceError>;
    async fn total_mrr(&self, tenant_id: Uuid) -> Result<i64, StatServiceError>;
    async fn total_mrr_chart(
        &self,
        request: MrrChartRequest,
    ) -> Result<MrrChartResponse, StatServiceError>;
    async fn mrr_breakdown(
        &self,
        request: MRRBreakdownRequest,
    ) -> Result<MRRBreakdown, StatServiceError>;
    async fn mrr_log(&self, request: MrrLogRequest) -> Result<MrrLogResponse, StatServiceError>;
}

pub struct PgStatsService {
    pool: Pool,
}
impl PgStatsService {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    pub async fn get_connection(&self) -> Result<Object, StatServiceError> {
        match self.pool.get().await {
            Ok(client) => Ok(client),
            Err(e) => {
                log::error!("Unable to get database connection : {}", e);
                Err(StatServiceError::InternalServerError(
                    "Unable to get database connection".to_string(),
                ))
            }
        }
    }

    pub async fn get_currency(&self, tenant_id: &Uuid) -> Result<String, StatServiceError> {
        let conn = self.get_connection().await?;
        let currency = db::tenants::get_tenant_by_id()
            .bind(&conn, tenant_id)
            .one()
            .await
            .map_err(|_| {
                StatServiceError::InternalServerError("Failed to query tenant".to_string())
            })?;

        let currency = currency.currency.as_str();
        Ok(currency.to_string())
    }
}
#[tonic::async_trait]
impl StatsService for PgStatsService {
    async fn net_revenue(&self, tenant_id: Uuid) -> Result<Trend, StatServiceError> {
        let conn = self.get_connection().await?;

        let trend = db::stats::query_revenue_trend()
            .params(
                &conn,
                &db::stats::QueryRevenueTrendParams {
                    period_days: 7,
                    tenant_id: tenant_id.clone(),
                },
            )
            .one()
            .await
            .map_err(|_| {
                StatServiceError::InternalServerError("Failed to query revenue trend".to_string())
            })?;

        let (change, percent) = calculate_trend(trend.total_ytd, trend.total_previous_period);

        Ok(Trend {
            current: trend.total_ytd,
            change_amount: change,
            change_percent: percent,
            positive_is_good: true,
            scope: TrendScope::Trend7d,
        })
    }

    async fn active_subscriptions(&self, tenant_id: Uuid) -> Result<i64, StatServiceError> {
        let conn = self.get_connection().await?;

        let trend = db::stats::count_active_subscriptions()
            .bind(&conn, &tenant_id)
            .one()
            .await
            .map_err(|_| {
                StatServiceError::InternalServerError(
                    "Failed to query total subscriptions".to_string(),
                )
            })?;

        Ok(trend)
    }

    async fn pending_invoices(&self, tenant_id: Uuid) -> Result<CountAndValue, StatServiceError> {
        let conn = self.get_connection().await?;

        // TODO currency
        let trend = db::stats::query_pending_invoices()
            .bind(&conn, &tenant_id)
            .one()
            .await
            .map_err(|_| {
                StatServiceError::InternalServerError(
                    "Failed to query pending invoices".to_string(),
                )
            })?;

        Ok(CountAndValue {
            count: trend.total,
            value: trend.total_cents.to_cents().map_err(|_| {
                StatServiceError::InternalServerError(
                    "Failed to convert pending invoice total cents".to_string(),
                )
            })?,
        })
    }

    async fn signups(&self, tenant_id: Uuid) -> Result<Trend, StatServiceError> {
        let conn = self.get_connection().await?;

        let trend = db::stats::new_signups_trend_30_days()
            .bind(&conn, &tenant_id)
            .one()
            .await
            .map_err(|_| {
                StatServiceError::InternalServerError("Failed to query signups trend".to_string())
            })?;

        let (change, percent) =
            calculate_trend(trend.total_last_30_days, trend.total_previous_30_days);

        Ok(Trend {
            current: trend.total_last_30_days,
            change_amount: change,
            change_percent: percent,
            positive_is_good: true,
            scope: TrendScope::Trend30d,
        })
    }

    async fn signups_sparkline(
        &self,
        tenant_id: Uuid,
    ) -> Result<SignupSparklineResponse, StatServiceError> {
        let conn = self.get_connection().await?;

        let chart_data = db::stats::daily_new_signups_30_days()
            .bind(&conn, &tenant_id)
            .all()
            .await
            .map_err(|_| {
                StatServiceError::InternalServerError("Failed to query signups dara".to_string())
            })?;

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

    async fn trial_conversion_rate(&self, tenant_id: Uuid) -> Result<f32, StatServiceError> {
        let conn = self.get_connection().await?;

        let all_time = db::stats::get_all_time_trial_conversion_rate()
            .bind(&conn, &tenant_id)
            .one()
            .await
            .map_err(|e| {
                StatServiceError::InternalServerError(format!(
                    "Failed to get trial conversion rate : {}",
                    e
                ))
            })?;

        Ok(all_time.round_dp(1).to_f32().unwrap_or(0.0))
    }
    async fn trial_conversion_rate_sparkline(
        &self,
        tenant_id: Uuid,
    ) -> Result<TrialConversionRateResponse, StatServiceError> {
        let conn = self.get_connection().await?;

        let chart_data = db::stats::query_trial_to_paid_conversion_over_time()
            .bind(&conn, &tenant_id)
            .all()
            .await
            .map_err(|_| {
                StatServiceError::InternalServerError(
                    "Failed to query trial conversion rate data".to_string(),
                )
            })?;

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

        let month_format = time::format_description::parse("[year]-[month]").unwrap();

        for dp in chart_data {
            let month_str = dp
                .month
                .format(&month_format)
                .map_err(|_| {
                    StatServiceError::InternalServerError("Failed to format month".to_string())
                })?
                .to_string();

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

    async fn top_revenue_by_customer(
        &self,
        request: RevenueByCustomerRequest,
    ) -> Result<Vec<RevenueByCustomer>, StatServiceError> {
        let conn = self.get_connection().await?;

        let currency = self.get_currency(&request.tenant_id).await?;

        let data = db::stats::top_revenue_per_customer()
            .bind(
                &conn,
                &request.tenant_id,
                &currency,
                &(request.limit as i64),
            )
            .all()
            .await
            .map_err(|_| {
                StatServiceError::InternalServerError(
                    "Failed to query top revenue by customer".to_string(),
                )
            })?;

        Ok(data
            .into_iter()
            .map(|d| RevenueByCustomer {
                customer_name: d.name,
                customer_id: d.id,
                revenue: d.total_revenue_cents,
            })
            .collect())
    }

    async fn total_mrr(&self, tenant_id: Uuid) -> Result<i64, StatServiceError> {
        let conn = self.get_connection().await?;

        let currency = self.get_currency(&tenant_id).await?;

        let total_mrr = db::stats::mrr_at_date()
            .bind(
                &conn,
                &time::OffsetDateTime::now_utc().date(),
                &tenant_id,
                &currency.clone(),
            )
            .one()
            .await
            .map_err(|_| {
                StatServiceError::InternalServerError("Failed to query mrr".to_string())
            })?;

        Ok(total_mrr)
    }
    async fn total_mrr_chart(
        &self,
        request: MrrChartRequest,
    ) -> Result<MrrChartResponse, StatServiceError> {
        let conn = self.get_connection().await?;

        let currency = self.get_currency(&request.tenant_id).await?;

        let total_data = db::stats::query_total_mrr()
            .params(
                &conn,
                &QueryTotalMrrParams {
                    tenant_id: request.tenant_id,
                    start_date: request.start_date,
                    end_date: request.end_date,
                    date_trunc: "day".to_string(),
                    currency: currency.clone(),
                },
            )
            .all()
            .await
            .map_err(|_| {
                StatServiceError::InternalServerError("Failed to query mrr chart".to_string())
            })?;

        let total_mrr_series = MrrChartSeries {
            name: "Total MRR".to_string(),
            code: "total_mrr".to_string(),
            plan: None,
            data: total_data
                .iter()
                .map(|d| MrrChartDataPoint {
                    x: d.period.to_string(),
                    data: MRRBreakdown {
                        new_business: CountAndValue {
                            count: d.new_business_count,
                            value: d.new_business_mrr,
                        },
                        expansion: CountAndValue {
                            count: d.expansion_count,
                            value: d.expansion_mrr,
                        },
                        contraction: CountAndValue {
                            count: d.contraction_count,
                            value: d.contraction_mrr,
                        },
                        churn: CountAndValue {
                            count: d.churn_count,
                            value: d.churn_mrr,
                        },
                        reactivation: CountAndValue {
                            count: d.reactivation_count,
                            value: d.reactivation_mrr,
                        },
                        net_new_mrr: d.net_new_mrr,
                    },
                })
                .collect(),
        };

        let mut series_map: HashMap<String, MrrChartSeries> = HashMap::new();
        if request.plans_id.is_some() {
            let plans_data = db::stats::query_total_mrr_by_plan()
                .params(
                    &conn,
                    &QueryTotalMrrByPlanParams {
                        tenant_id: request.tenant_id,
                        plan_ids: request.plans_id.unwrap(),
                        start_date: request.start_date,
                        end_date: request.end_date,
                        date_trunc: "day".to_string(),
                        currency: currency.clone(),
                    },
                )
                .all()
                .await
                .map_err(|_| {
                    StatServiceError::InternalServerError(
                        "Failed to query mrr chart by plan".to_string(),
                    )
                })?;

            let day_format = time::format_description::parse("[year]-[month]-[day]").unwrap();

            for data in plans_data {
                let data_point = MrrChartDataPoint {
                    x: data
                        .period
                        .format(&day_format)
                        .map_err(|_| {
                            StatServiceError::InternalServerError(
                                "Failed to format month".to_string(),
                            )
                        })?
                        .to_string(),
                    data: MRRBreakdown {
                        new_business: CountAndValue {
                            count: data.new_business_count,
                            value: data.new_business_mrr,
                        },
                        expansion: CountAndValue {
                            count: data.expansion_count,
                            value: data.expansion_mrr,
                        },
                        contraction: CountAndValue {
                            count: data.contraction_count,
                            value: data.contraction_mrr,
                        },
                        churn: CountAndValue {
                            count: data.churn_count,
                            value: data.churn_mrr,
                        },
                        reactivation: CountAndValue {
                            count: data.reactivation_count,
                            value: data.reactivation_mrr,
                        },
                        net_new_mrr: data.net_new_mrr,
                    },
                };

                series_map
                    .entry(data.name.clone())
                    .or_insert_with(|| MrrChartSeries {
                        name: data.name.clone(),
                        code: format!("mrr_breakdown_plan_{}", data.id),
                        plan: Some(Plan {
                            id: data.id,
                            name: data.name.clone(),
                        }),
                        data: vec![],
                    })
                    .data
                    .push(data_point);
            }
        }
        let mut series: Vec<MrrChartSeries> = series_map.into_iter().map(|(_, v)| v).collect();
        series.push(total_mrr_series);

        Ok(MrrChartResponse { series })
    }

    async fn mrr_breakdown(
        &self,
        request: MRRBreakdownRequest,
    ) -> Result<MRRBreakdown, StatServiceError> {
        let conn = self.get_connection().await?;

        let now = time::OffsetDateTime::now_utc().date();
        let (start_date, end_date) = request.scope.to_date_range(now);
        let breakdown = db::stats::get_mrr_breakdown()
            .params(
                &conn,
                &GetMrrBreakdownParams {
                    tenant_id: request.tenant_id,
                    start_date,
                    end_date,
                },
            )
            .one()
            .await
            .map_err(|_| {
                StatServiceError::InternalServerError("Failed to query mrr breakdown".to_string())
            })?;

        Ok(MRRBreakdown {
            new_business: CountAndValue {
                count: breakdown.new_business_count,
                value: breakdown.new_business_mrr,
            },
            expansion: CountAndValue {
                count: breakdown.expansion_count,
                value: breakdown.expansion_mrr,
            },
            contraction: CountAndValue {
                count: breakdown.contraction_count,
                value: breakdown.contraction_mrr,
            },
            churn: CountAndValue {
                count: breakdown.churn_count,
                value: breakdown.churn_mrr,
            },
            reactivation: CountAndValue {
                count: breakdown.reactivation_count,
                value: breakdown.reactivation_mrr,
            },
            net_new_mrr: breakdown.net_new_mrr,
        })
    }

    async fn mrr_log(&self, request: MrrLogRequest) -> Result<MrrLogResponse, StatServiceError> {
        let conn = self.get_connection().await?;

        let data = db::stats::get_last_mrr_movements()
            .bind(
                &conn,
                &request.tenant_id,
                &request
                    .before
                    .map(|s| {
                        s.parse().map_err(|_| {
                            StatServiceError::InternalServerError(
                                "Invalid before cursor".to_string(),
                            )
                        })
                    })
                    .transpose()?,
                &request
                    .after
                    .map(|s| {
                        s.parse().map_err(|_| {
                            StatServiceError::InternalServerError(
                                "Invalid after cursor".to_string(),
                            )
                        })
                    })
                    .transpose()?,
                &10i64,
            )
            .all()
            .await
            .map_err(|_| {
                StatServiceError::InternalServerError("Failed to query mrr log".to_string())
            })?;

        Ok(MrrLogResponse {
            cursor: "TODO".to_string(),
            entries: data
                .into_iter()
                .map(|d| MrrLogEntry {
                    applies_to: d.applies_to,
                    created_at: d.created_at.assume_utc(),
                    customer_id: d.customer_id.to_string(),
                    customer_name: d.customer_name,
                    subscription_id: d.subscription_id.to_string(),
                    plan_name: d.plan_name,
                    description: d.description,
                    mrr_type: map_movement_type(d.movement_type),
                })
                .collect(),
        })
    }
}

fn map_movement_type(m: db::MrrMovementType) -> MrrMovementType {
    match m {
        db::MrrMovementType::NEW_BUSINESS => MrrMovementType::NewBusiness,
        db::MrrMovementType::EXPANSION => MrrMovementType::Expansion,
        db::MrrMovementType::CONTRACTION => MrrMovementType::Contraction,
        db::MrrMovementType::CHURN => MrrMovementType::Churn,
        db::MrrMovementType::REACTIVATION => MrrMovementType::Reactivation,
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
