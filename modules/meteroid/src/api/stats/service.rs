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

use crate::api::shared;
use crate::api::stats::mapping::trend_to_server;

use common_grpc::middleware::server::auth::RequestExt;

use meteroid_grpc::meteroid::api::stats::v1::mrr_chart_series;
use meteroid_grpc::meteroid::api::stats::v1::revenue_chart_series;
use meteroid_grpc::meteroid::api::stats::v1::trial_conversion_series;
use meteroid_store::domain::stats::RevenueByCustomerRequest;

#[tonic::async_trait]
impl StatsService for StatsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn general_stats(
        &self,
        request: Request<GeneralStatsRequest>,
    ) -> Result<Response<GeneralStatsResponse>, Status> {
        let tenant_id = request.tenant()?;

        let (
            net_revenue_res,
            active_subscriptions,
            pending_invoices_res,
            signups_res,
            trial_conversion_res,
            total_mrr_res,
        ) = tokio::try_join!(
            self.store.net_revenue(tenant_id),
            self.store.active_subscriptions(tenant_id),
            self.store.pending_invoices(tenant_id),
            self.store.signups(tenant_id),
            self.store.trial_conversion_rate(tenant_id),
            self.store.total_mrr(tenant_id)
        )
        .map_err(|e| Status::internal(format!("Failed to fetch stats: {e}")))?;

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

        let now = chrono::Utc::now().naive_utc().date();
        let start_date = req
            .start_date
            .and_then(shared::mapping::date::chrono_from_proto)
            .unwrap_or(now.checked_sub_months(Months::new(12)).unwrap());

        let end_date = req
            .end_date
            .and_then(shared::mapping::date::chrono_from_proto)
            .unwrap_or(now);

        let plans_id: Option<Vec<common_domain::ids::PlanId>> = if req.plans_id.is_empty() {
            None
        } else {
            Some(
                req.plans_id
                    .iter()
                    .map(common_domain::ids::PlanId::from_proto)
                    .collect::<Result<Vec<_>, _>>()?,
            )
        };

        let mrr_chart = self
            .store
            .total_mrr_chart(meteroid_store::domain::stats::MrrChartRequest {
                tenant_id,
                start_date,
                end_date,
                plans_id,
            })
            .await
            .map_err(|e| Status::internal(format!("Failed to fetch mrr chart: {e}")))?;

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

        let now = chrono::Utc::now().naive_utc().date();
        let start_date = req
            .start_date
            .and_then(shared::mapping::date::chrono_from_proto)
            .unwrap_or(now.checked_sub_months(Months::new(12)).unwrap());

        let end_date = req
            .end_date
            .and_then(shared::mapping::date::chrono_from_proto)
            .unwrap_or(now);

        let plans_id: Option<Vec<common_domain::ids::PlanId>> = if req.plans_id.is_empty() {
            None
        } else {
            Some(
                req.plans_id
                    .iter()
                    .map(common_domain::ids::PlanId::from_proto)
                    .collect::<Result<Vec<_>, _>>()?,
            )
        };

        let revenue_chart = self
            .store
            .total_revenue_chart(meteroid_store::domain::stats::RevenueChartRequest {
                tenant_id,
                start_date,
                end_date,
                plans_id,
            })
            .await
            .map_err(|e| Status::internal(format!("Failed to fetch revenue chart: {e}")))?;

        let series = revenue_chart
            .series
            .into_iter()
            .map(|series| RevenueChartSeries {
                name: series.name,
                code: series.code,
                data: series
                    .data
                    .into_iter()
                    .map(|dp| revenue_chart_series::DataPoint {
                        x: dp.x,
                        revenue: dp.revenue,
                        daily_revenue: dp.daily_revenue,
                    })
                    .collect(),
            })
            .collect();

        Ok(Response::new(RevenueChartResponse { series }))
    }

    #[tracing::instrument(skip_all)]
    async fn mrr_breakdown(
        &self,
        request: Request<MrrBreakdownRequest>,
    ) -> Result<Response<MrrBreakdownResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let now = chrono::Utc::now().naive_utc().date();
        let start_date = req
            .start_date
            .and_then(shared::mapping::date::chrono_from_proto)
            .unwrap_or(now.checked_sub_months(Months::new(1)).unwrap());

        let end_date = req
            .end_date
            .and_then(shared::mapping::date::chrono_from_proto)
            .unwrap_or(now);

        let mrr_breakdown = self
            .store
            .mrr_breakdown(meteroid_store::domain::stats::MRRBreakdownRequest {
                tenant_id,
                start_date,
                end_date,
            })
            .await
            .map_err(|e| Status::internal(format!("Failed to fetch mrr breakdown: {e}")))?;

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
        let req = request.into_inner();

        let mrr_log = self
            .store
            .mrr_log(meteroid_store::domain::stats::MrrLogRequest {
                tenant_id,
                before: req.before,
                after: req.after,
            })
            .await
            .map_err(|e| Status::internal(format!("Failed to fetch mrr log: {e}")))?;

        Ok(Response::new(MrrLogResponse {
            entries: mrr_log
                .entries
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

        let top_revenue_by_customer = self
            .store
            .top_revenue_by_customer(RevenueByCustomerRequest {
                tenant_id,
                limit: req.count,
                currency: None, // TODO decide between approx via usd or currency select
            })
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
