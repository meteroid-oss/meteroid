use chrono::Months;
use tonic::{Request, Response, Status};

use super::{StatsServiceComponents, mapping};
use meteroid_grpc::meteroid::api::stats::v1 as grpc;
use meteroid_grpc::meteroid::api::stats::v1::{
    GeneralStatsRequest, GeneralStatsResponse, MrrBreakdownRequest, MrrBreakdownResponse,
    MrrChartRequest, MrrChartResponse, MrrChartSeries, MrrLogRequest, MrrLogResponse,
    RevenueChartRequest, RevenueChartResponse, RevenueChartSeries, SignupSeries,
    SignupSparklineRequest, SignupSparklineRequestResponse, TopRevenueByCustomerRequest,
    TopRevenueByCustomerResponse, TrialConversionMetaDataPoint,
    TrialConversionRateSparklineRequest, TrialConversionRateSparklineResponse,
    TrialConversionSeries, general_stats_response, signup_series,
    stats_service_server::StatsService,
};

use meteroid_store::repositories::stats::StatsInterface;
use meteroid_store::repositories::tenants::TenantInterface;

use crate::stats::mapping::trend_to_server;
use meteroid_grpc_mapping::shared;

use common_grpc::middleware::server::auth::RequestExt;

use meteroid_grpc::meteroid::api::stats::v1::mrr_chart_series;
use meteroid_grpc::meteroid::api::stats::v1::revenue_chart_series;
use meteroid_grpc::meteroid::api::stats::v1::trial_conversion_series;
use meteroid_store::domain::stats::{
    build_mrr_chart_by_plan, build_revenue_chart, build_total_mrr_chart, revenue_trend,
    sum_mrr_breakdown,
};

/// Trailing window (days) the dashboard net-revenue trend compares.
const REVENUE_TREND_DAYS: u32 = 7;

/// The dashboard MRR log feed length (the legacy endpoint's fixed page size).
const MRR_LOG_LIMIT: u32 = 10;

#[tonic::async_trait]
impl StatsService for StatsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn general_stats(
        &self,
        request: Request<GeneralStatsRequest>,
    ) -> Result<Response<GeneralStatsResponse>, Status> {
        let tenant_id = request.tenant()?;

        let reporting = self
            .store
            .get_reporting_currency_by_tenant_id(tenant_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to resolve reporting currency: {e}")))?;
        let code = reporting.code;

        let today = common_domain::clock::now().naive_utc().date();
        // The trend needs YTD plus two trailing comparison windows in one series.
        let trend_from = meteroid_store::utils::datetime::start_of_year(today)
            .min(today - chrono::Days::new(2 * REVENUE_TREND_DAYS as u64));

        // Revenue trend & total MRR come from the edition's revenue stats; the operational
        // counters read live Postgres tables.
        let (
            revenue_daily,
            mrr_today,
            active_subscriptions,
            pending_invoices_res,
            signups_res,
            trial_conversion_res,
        ) = tokio::try_join!(
            self.revenue_stats
                .revenue_daily(tenant_id, trend_from, today, &[], code),
            self.revenue_stats
                .mrr_timeseries(tenant_id, today, today, &[], false, code),
            self.store.active_subscriptions(tenant_id),
            self.store.pending_invoices(tenant_id),
            self.store.signups(tenant_id),
            self.store.trial_conversion_rate(tenant_id),
        )
        .map_err(|e| Status::internal(format!("Failed to fetch stats: {e}")))?;

        let net_revenue_res = revenue_trend(&revenue_daily, today, REVENUE_TREND_DAYS);
        let total_mrr_res = mrr_today.total_mrr_cents();

        Ok(Response::new(GeneralStatsResponse {
            total_net_revenue: Some(general_stats_response::TotalNetRevenue {
                trend: Some(trend_to_server(&net_revenue_res)),
            }),
            total_active_subscriptions: Some(general_stats_response::TotalActiveSubscriptions {
                count: active_subscriptions,
            }),
            pending_invoices: Some(general_stats_response::PendingInvoices {
                count: i64::from(pending_invoices_res.count),
                value_cents: pending_invoices_res.value,
            }),
            signups: Some(general_stats_response::Signups {
                count: signups_res.current,
            }),
            total_mrr: Some(general_stats_response::TotalMrr {
                value_cents: total_mrr_res,
            }),
            trial_conversion: Some(general_stats_response::TrialConversion {
                rate_percent: trial_conversion_res,
            }),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn total_mrr_chart(
        &self,
        request: Request<MrrChartRequest>,
    ) -> Result<Response<MrrChartResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let now = common_domain::clock::now().naive_utc().date();
        let start_date = req
            .start_date
            .and_then(shared::mapping::date::chrono_from_proto)
            .unwrap_or(now.checked_sub_months(Months::new(12)).unwrap());

        let end_date = req
            .end_date
            .and_then(shared::mapping::date::chrono_from_proto)
            .unwrap_or(now);

        let plans_id: Vec<common_domain::ids::PlanId> = req
            .plans_id
            .iter()
            .map(common_domain::ids::PlanId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        let reporting = self
            .store
            .get_reporting_currency_by_tenant_id(tenant_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to resolve reporting currency: {e}")))?;

        let split_by_plan = !plans_id.is_empty();
        let rows = self
            .revenue_stats
            .mrr_timeseries(
                tenant_id,
                start_date,
                end_date,
                &plans_id,
                split_by_plan,
                reporting.code,
            )
            .await
            .map_err(|e| Status::internal(format!("Failed to fetch mrr chart: {e}")))?;

        let mrr_chart = if split_by_plan {
            // Resolve names from Postgres (tenant-scoped) so every selected plan
            // gets a series, movements or not — matching the legacy chart.
            use meteroid_store::repositories::PlansInterface;
            let mut plans = Vec::with_capacity(plans_id.len());
            for plan_id in &plans_id {
                let overview = self
                    .store
                    .get_plan_overview(*plan_id, tenant_id)
                    .await
                    .map_err(|e| Status::internal(format!("Failed to resolve plan: {e}")))?;
                plans.push(meteroid_store::domain::stats::PlanBrief {
                    id: *plan_id,
                    name: overview.name,
                });
            }
            build_mrr_chart_by_plan(&rows, &plans, start_date, end_date)
        } else {
            build_total_mrr_chart(&rows, start_date, end_date)
        };

        let series = mrr_chart
            .series
            .into_iter()
            .map(|series| MrrChartSeries {
                name: series.name,
                code: series.code,
                plan: series.plan.map(|plan| mrr_chart_series::Plan {
                    id: plan.id.to_string(),
                    name: plan.name,
                }),
                data: series
                    .data
                    .into_iter()
                    .map(|dp| mrr_chart_series::DataPoint {
                        x: dp.x,
                        data: Some(mapping::mrr_breakdown_to_server(&dp.data)),
                    })
                    .collect(),
            })
            .collect();

        Ok(Response::new(MrrChartResponse { series }))
    }

    #[tracing::instrument(skip_all)]
    async fn total_revenue_chart(
        &self,
        request: Request<RevenueChartRequest>,
    ) -> Result<Response<RevenueChartResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let now = common_domain::clock::now().naive_utc().date();
        let start_date = req
            .start_date
            .and_then(shared::mapping::date::chrono_from_proto)
            .unwrap_or(now.checked_sub_months(Months::new(12)).unwrap());

        let end_date = req
            .end_date
            .and_then(shared::mapping::date::chrono_from_proto)
            .unwrap_or(now);

        let plans_id: Vec<common_domain::ids::PlanId> = req
            .plans_id
            .iter()
            .map(common_domain::ids::PlanId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        let reporting = self
            .store
            .get_reporting_currency_by_tenant_id(tenant_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to resolve reporting currency: {e}")))?;

        let daily = self
            .revenue_stats
            .revenue_daily(tenant_id, start_date, end_date, &plans_id, reporting.code)
            .await
            .map_err(|e| Status::internal(format!("Failed to fetch revenue chart: {e}")))?;

        let series = vec![RevenueChartSeries {
            name: "Total Revenue".to_string(),
            code: "total_revenue".to_string(),
            data: build_revenue_chart(&daily, start_date, end_date)
                .into_iter()
                .map(|dp| revenue_chart_series::DataPoint {
                    x: dp.x,
                    revenue: dp.revenue,
                    daily_revenue: dp.daily_revenue,
                })
                .collect(),
        }];

        Ok(Response::new(RevenueChartResponse { series }))
    }

    #[tracing::instrument(skip_all)]
    async fn mrr_breakdown(
        &self,
        request: Request<MrrBreakdownRequest>,
    ) -> Result<Response<MrrBreakdownResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let now = common_domain::clock::now().naive_utc().date();
        let start_date = req
            .start_date
            .and_then(shared::mapping::date::chrono_from_proto)
            .unwrap_or(now.checked_sub_months(Months::new(1)).unwrap());

        let end_date = req
            .end_date
            .and_then(shared::mapping::date::chrono_from_proto)
            .unwrap_or(now);

        let reporting = self
            .store
            .get_reporting_currency_by_tenant_id(tenant_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to resolve reporting currency: {e}")))?;

        let rows = self
            .revenue_stats
            .mrr_timeseries(tenant_id, start_date, end_date, &[], false, reporting.code)
            .await
            .map_err(|e| Status::internal(format!("Failed to fetch mrr breakdown: {e}")))?;

        let mrr_breakdown = sum_mrr_breakdown(&rows.points);

        Ok(Response::new(MrrBreakdownResponse {
            mmr_breakdown: Some(mapping::mrr_breakdown_to_server(&mrr_breakdown)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn mrr_log(
        &self,
        request: Request<MrrLogRequest>,
    ) -> Result<Response<MrrLogResponse>, Status> {
        let tenant_id = request.tenant()?;
        // before/after cursors were never implemented by the legacy endpoint
        // (it always served the latest page); the feed keeps that contract.
        let _ = request.get_ref();

        let entries = self
            .revenue_stats
            .mrr_log(tenant_id, MRR_LOG_LIMIT, None)
            .await
            .map_err(|e| Status::internal(format!("Failed to fetch mrr log: {e}")))?;

        Ok(Response::new(MrrLogResponse {
            entries: entries
                .into_iter()
                .map(|entry| grpc::MrrLogEntry {
                    mrr_type: mapping::map_mrr_type(entry.mrr_type).into(),
                    customer_id: entry.customer_id,
                    customer_name: entry.customer_name,
                    applies_to: Some(shared::mapping::date::chrono_to_proto(entry.applies_to)),
                    created_at: Some(shared::mapping::datetime::chrono_to_timestamp(
                        entry.created_at,
                    )),
                    description: entry.description,
                    plan_name: entry.plan_name,
                    subscription_id: entry.subscription_id,
                    mrr_change: entry.mrr_change,
                    currency: entry.currency,
                })
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn signup_sparkline(
        &self,
        request: Request<SignupSparklineRequest>,
    ) -> Result<Response<SignupSparklineRequestResponse>, Status> {
        let tenant_id = request.tenant()?;

        let res = self
            .store
            .signups_sparkline(tenant_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to fetch signup sparkline: {e}")))?;

        Ok(Response::new(SignupSparklineRequestResponse {
            series: Some(SignupSeries {
                name: res.series.name,
                code: res.series.code,
                data: res
                    .series
                    .data
                    .into_iter()
                    .map(|dp| signup_series::DataPoint {
                        x: dp.x,
                        total: dp.total,
                        delta: dp.delta,
                    })
                    .collect(),
            }),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn trial_conversion_rate_sparkline(
        &self,
        request: Request<TrialConversionRateSparklineRequest>,
    ) -> Result<Response<TrialConversionRateSparklineResponse>, Status> {
        let tenant_id = request.tenant()?;

        let res = self
            .store
            .trial_conversion_rate_sparkline(tenant_id)
            .await
            .map_err(|e| {
                Status::internal(format!(
                    "Failed to fetch trial conversion rate sparkline: {e}"
                ))
            })?;

        Ok(Response::new(TrialConversionRateSparklineResponse {
            series: res
                .series
                .into_iter()
                .map(|series| TrialConversionSeries {
                    name: series.name,
                    code: series.code,
                    data: series
                        .data
                        .into_iter()
                        .map(|dp| trial_conversion_series::DataPoint {
                            x: dp.x,
                            conversion_rate: dp.conversion_rate,
                            conversions: dp.conversions,
                        })
                        .collect(),
                })
                .collect(),
            metadata: res
                .metadata
                .into_iter()
                .map(|metadata| TrialConversionMetaDataPoint {
                    x: metadata.x,
                    total_trials: metadata.total_trials,
                })
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn top_revenue_by_customer(
        &self,
        request: Request<TopRevenueByCustomerRequest>,
    ) -> Result<Response<TopRevenueByCustomerResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let reporting = self
            .store
            .get_reporting_currency_by_tenant_id(tenant_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to resolve reporting currency: {e}")))?;

        let top_revenue_by_customer = self
            .revenue_stats
            .top_revenue_by_customer(tenant_id, req.count, reporting.code)
            .await
            .map_err(|e| {
                Status::internal(format!("Failed to fetch top revenue by customer: {e}"))
            })?;

        let top_revenue_by_customer = top_revenue_by_customer
            .into_iter()
            .map(|customer| grpc::RevenueByCustomer {
                customer_id: customer.customer_id.to_string(),
                customer_name: customer.customer_name,
                revenue_ytd: customer.revenue_ytd,
                revenue_all_time: customer.revenue_all_time,
            })
            .collect();

        Ok(Response::new(TopRevenueByCustomerResponse {
            revenue_by_customer: top_revenue_by_customer,
        }))
    }
}
