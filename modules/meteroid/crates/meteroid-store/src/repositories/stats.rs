use crate::domain::stats::{
    CountAndValue, MRRBreakdown, MRRBreakdownRequest, MrrChartDataPoint, MrrChartRequest,
    MrrChartResponse, MrrChartSeries, MrrLogEntry, MrrLogRequest, MrrLogResponse, MrrMovementType,
    PlanBrief, RevenueByCustomer, RevenueByCustomerRequest, RevenueChartDataPoint,
    RevenueChartRequest, RevenueChartResponse, RevenueChartSeries, SignupDataPoint, SignupSeries,
    SignupSparklineResponse, Trend, TrendScope, TrialConversionDataPoint,
    TrialConversionMetaDataPoint, TrialConversionRateResponse, TrialConversionSeries,
};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::TenantId;
use common_utils::decimals::ToSubunit;
use diesel_models::stats::{
    ActiveSubscriptionsCountRow, CustomerTopRevenueRow, DailyNewSignups90DaysRow,
    LastMrrMovementsRow, MrrBreakdownRow, NewSignupsTrend90DaysRow, PendingInvoicesTotalRow,
    RevenueChartRow, RevenueTrendRow, SubscriptionTrialConversionRateRow,
    SubscriptionTrialToPaidConversionRow, TotalMrrByPlanRow, TotalMrrChartRow, TotalMrrRow,
};
use diesel_models::tenants::TenantRow;
use error_stack::Report;
use rust_decimal::prelude::ToPrimitive;
use std::collections::HashMap;

#[async_trait::async_trait]
pub trait StatsInterface {
    async fn net_revenue(&self, tenant_id: TenantId) -> StoreResult<Trend>;
    async fn active_subscriptions(&self, tenant_id: TenantId) -> StoreResult<i64>;
    async fn pending_invoices(&self, tenant_id: TenantId) -> StoreResult<CountAndValue>;
    async fn signups(&self, tenant_id: TenantId) -> StoreResult<Trend>;
    async fn signups_sparkline(&self, tenant_id: TenantId) -> StoreResult<SignupSparklineResponse>;
    async fn trial_conversion_rate(&self, tenant_id: TenantId) -> StoreResult<f32>;
    async fn trial_conversion_rate_sparkline(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<TrialConversionRateResponse>;
    async fn top_revenue_by_customer(
        &self,
        request: RevenueByCustomerRequest,
    ) -> StoreResult<Vec<RevenueByCustomer>>;
    async fn total_mrr(&self, tenant_id: TenantId) -> StoreResult<i64>;
    async fn total_mrr_chart(&self, request: MrrChartRequest) -> StoreResult<MrrChartResponse>;
    async fn total_revenue_chart(
        &self,
        request: RevenueChartRequest,
    ) -> StoreResult<RevenueChartResponse>;
    async fn mrr_breakdown(&self, request: MRRBreakdownRequest) -> StoreResult<MRRBreakdown>;
    async fn mrr_log(&self, request: MrrLogRequest) -> StoreResult<MrrLogResponse>;
}

#[async_trait::async_trait]
impl StatsInterface for Store {
    async fn net_revenue(&self, tenant_id: TenantId) -> StoreResult<Trend> {
        let mut conn = self.get_conn().await?;

        let trend = RevenueTrendRow::get(&mut conn, 7, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let (change, percent) = calculate_trend(trend.total_ytd, trend.total_previous_period);

        Ok(Trend {
            current: trend.total_ytd,
            change_amount: change,
            change_percent: percent,
            positive_is_good: true,
            scope: TrendScope::Trend7d,
        })
    }

    async fn active_subscriptions(&self, tenant_id: TenantId) -> StoreResult<i64> {
        let mut conn = self.get_conn().await?;

        ActiveSubscriptionsCountRow::get(&mut conn, tenant_id, None)
            .await
            .map_err(Into::into)
            .map(|x| x.count.into())
    }

    async fn pending_invoices(&self, tenant_id: TenantId) -> StoreResult<CountAndValue> {
        let mut conn = self.get_conn().await?;

        let trend = PendingInvoicesTotalRow::get(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let currency = self
            .internal
            .get_reporting_currency_by_tenant_id(&mut conn, tenant_id)
            .await?;

        Ok(CountAndValue {
            count: trend.total,
            value: trend.total_cents.to_subunit_opt(currency.precision).ok_or(
                StoreError::InvalidArgument(
                    "Failed to convert pending invoice total cents".to_string(),
                ),
            )?,
        })
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

    async fn top_revenue_by_customer(
        &self,
        request: RevenueByCustomerRequest,
    ) -> StoreResult<Vec<RevenueByCustomer>> {
        let mut conn = self.get_conn().await?;

        let reporting_currency = match request.currency {
            Some(currency) => currency,
            None => {
                TenantRow::find_by_id(&mut conn, request.tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .reporting_currency
            }
        };

        let data = CustomerTopRevenueRow::list(
            &mut conn,
            request.tenant_id,
            reporting_currency.as_str(),
            request.limit as i32,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(data
            .into_iter()
            .map(|d| RevenueByCustomer {
                customer_name: d.name,
                customer_id: d.id,
                revenue: d.total_revenue_cents,
                currency: d.currency,
            })
            .collect())
    }

    async fn total_mrr(&self, tenant_id: TenantId) -> StoreResult<i64> {
        let mut conn = self.get_conn().await?;

        TotalMrrRow::get(&mut conn, tenant_id, chrono::Utc::now().naive_utc().date())
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|x| x.total_net_mrr_cents)
    }

    async fn total_mrr_chart(&self, request: MrrChartRequest) -> StoreResult<MrrChartResponse> {
        let mut conn = self.get_conn().await?;

        // If plans are selected, return plan-specific series only
        if let Some(plan_ids) = &request.plans_id
            && !plan_ids.is_empty() {
                // Convert PlanId to Uuid for the query
                let plan_uuids: Vec<uuid::Uuid> = plan_ids.iter().map(|id| **id).collect();
                let plans_data = TotalMrrByPlanRow::list(
                    &mut conn,
                    request.tenant_id,
                    &plan_uuids,
                    request.start_date,
                    request.end_date,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                let mut series_map: HashMap<String, MrrChartSeries> = HashMap::new();
                for data in plans_data {
                    let data_point = MrrChartDataPoint {
                        x: data.date.format("%Y-%m-%d").to_string(),
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
                            total_net_mrr: data.total_net_mrr,
                        },
                    };

                    series_map
                        .entry(data.plan_name.clone())
                        .or_insert_with(|| MrrChartSeries {
                            name: data.plan_name.clone(),
                            code: format!("mrr_breakdown_plan_{}", data.plan_id),
                            plan: Some(PlanBrief {
                                id: data.plan_id,
                                name: data.plan_name.clone(),
                            }),
                            data: vec![],
                        })
                        .data
                        .push(data_point);
                }

                let series: Vec<MrrChartSeries> = series_map.into_values().collect();
                return Ok(MrrChartResponse { series });
            }

        // No plans selected - return total MRR across all plans
        let total = TotalMrrChartRow::list(
            &mut conn,
            request.tenant_id,
            request.start_date,
            request.end_date,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let total_mrr_series = MrrChartSeries {
            name: "Total MRR".to_string(),
            code: "total_mrr".to_string(),
            plan: None,
            data: total
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
                        total_net_mrr: d.total_net_mrr,
                    },
                })
                .collect(),
        };

        Ok(MrrChartResponse {
            series: vec![total_mrr_series],
        })
    }

    async fn total_revenue_chart(
        &self,
        request: RevenueChartRequest,
    ) -> StoreResult<RevenueChartResponse> {
        let mut conn = self.get_conn().await?;

        // Convert PlanId to Uuid for the query
        let plan_uuids: Option<Vec<uuid::Uuid>> = request
            .plans_id
            .as_ref()
            .map(|ids| ids.iter().map(|id| **id).collect());

        let data = RevenueChartRow::list(
            &mut conn,
            request.tenant_id,
            request.start_date,
            request.end_date,
            plan_uuids.as_ref(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let series = RevenueChartSeries {
            name: "Total Revenue".to_string(),
            code: "total_revenue".to_string(),
            data: data
                .iter()
                .map(|d| RevenueChartDataPoint {
                    x: d.period.to_string(),
                    revenue: d.total_revenue,
                })
                .collect(),
        };

        Ok(RevenueChartResponse {
            series: vec![series],
        })
    }

    async fn mrr_breakdown(&self, request: MRRBreakdownRequest) -> StoreResult<MRRBreakdown> {
        let mut conn = self.get_conn().await?;

        let breakdown = MrrBreakdownRow::get(
            &mut conn,
            request.tenant_id,
            request.start_date,
            request.end_date,
        )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        match breakdown {
            None => Ok(MRRBreakdown {
                new_business: CountAndValue { count: 0, value: 0 },
                expansion: CountAndValue { count: 0, value: 0 },
                contraction: CountAndValue { count: 0, value: 0 },
                churn: CountAndValue { count: 0, value: 0 },
                reactivation: CountAndValue { count: 0, value: 0 },
                net_new_mrr: 0,
                total_net_mrr: 0,
            }),
            Some(breakdown) => Ok(MRRBreakdown {
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
                total_net_mrr: 0,
            }),
        }
    }

    async fn mrr_log(&self, request: MrrLogRequest) -> StoreResult<MrrLogResponse> {
        let mut conn = self.get_conn().await?;

        let data = LastMrrMovementsRow::list(
            &mut conn,
            request.tenant_id,
            request
                .before
                .map(|s| {
                    s.parse().map_err(|_| {
                        StoreError::InvalidArgument("Invalid before cursor".to_string())
                    })
                })
                .transpose()?,
            request
                .after
                .map(|s| {
                    s.parse().map_err(|_| {
                        StoreError::InvalidArgument("Invalid after cursor".to_string())
                    })
                })
                .transpose()?,
            10,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(MrrLogResponse {
            cursor: "TODO".to_string(),
            entries: data
                .into_iter()
                .map(|d| MrrLogEntry {
                    applies_to: d.applies_to,
                    created_at: d.created_at,
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

fn map_movement_type(m: diesel_models::enums::MrrMovementType) -> MrrMovementType {
    match m {
        diesel_models::enums::MrrMovementType::NewBusiness => MrrMovementType::NewBusiness,
        diesel_models::enums::MrrMovementType::Expansion => MrrMovementType::Expansion,
        diesel_models::enums::MrrMovementType::Contraction => MrrMovementType::Contraction,
        diesel_models::enums::MrrMovementType::Churn => MrrMovementType::Churn,
        diesel_models::enums::MrrMovementType::Reactivation => MrrMovementType::Reactivation,
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
