

use tonic::{Request, Response, Status};

use crate::{
    api::services::utils::{parse_uuid},
    parse_uuid,
};

use super::{mapping, StatsServiceComponents};
use meteroid_grpc::meteroid::api::stats::v1 as grpc;
use meteroid_grpc::meteroid::api::stats::v1::{general_stats_response, signup_series, stats_service_server::StatsService, GeneralStatsRequest, GeneralStatsResponse, MrrBreakdownRequest, MrrBreakdownResponse, MrrChartRequest, MrrChartResponse, MrrChartSeries, MrrLogRequest, MrrLogResponse, SignupSeries, SignupSparklineRequest, SignupSparklineRequestResponse, TopRevenueByCustomerRequest, TopRevenueByCustomerResponse, TrialConversionRateSparklineRequest, TrialConversionRateSparklineResponse, TrialConversionSeries, TrialConversionMetaDataPoint, MrrBreakdownScope};



use crate::api::services::shared;
use crate::api::services::stats::mapping::trend_to_server;

use crate::services::stats::stats_service;
use crate::services::stats::stats_service::{
    RevenueByCustomerRequest,
};

use uuid::Uuid;
use common_grpc::middleware::server::auth::RequestExt;

use meteroid_grpc::meteroid::api::stats::v1::mrr_chart_series;
use meteroid_grpc::meteroid::api::stats::v1::trial_conversion_series;


#[tonic::async_trait]
impl StatsService for StatsServiceComponents {
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
            self.stats_service.net_revenue(tenant_id),
            self.stats_service.active_subscriptions(tenant_id),
            self.stats_service.pending_invoices(tenant_id),
            self.stats_service.signups(tenant_id),
            self.stats_service.trial_conversion_rate(tenant_id),
            self.stats_service.total_mrr(tenant_id)
        )
        .map_err(|e| Status::internal(format!("Failed to fetch stats: {}", e)))?;

        Ok(Response::new(GeneralStatsResponse {
            total_net_revenue: Some(general_stats_response::TotalNetRevenue {
                trend: Some(trend_to_server(&net_revenue_res)),
            }),
            total_active_subscriptions: Some(general_stats_response::TotalActiveSubscriptions {
                count: active_subscriptions,
            }),
            pending_invoices: Some(general_stats_response::PendingInvoices {
                count: pending_invoices_res.count,
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

    async fn total_mrr_chart(
        &self,
        request: Request<MrrChartRequest>,
    ) -> Result<Response<MrrChartResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let now = time::OffsetDateTime::now_utc().date();
        let start_date = req
            .start_date
            .map(shared::mapping::date::from_proto)
            .transpose()
            .map_err(|e| Status::invalid_argument(format!("Failed to parse start date: {}", e)))?
            .unwrap_or(now.replace_year(now.year() - 1).unwrap());

        let end_date = req
            .end_date
            .map(shared::mapping::date::from_proto)
            .transpose()
            .map_err(|e| Status::invalid_argument(format!("Failed to parse end date: {}", e)))?
            .unwrap_or(now);

        let plans_id = if req.plans_id.is_empty() {
            None
        } else {
            let parsed: Vec<Uuid> =  req.plans_id.into_iter()
                .map(|plan_id|
                    parse_uuid!(&plan_id)
                ).collect::<Result<Vec<Uuid>, Status>>()?;
            Some(parsed)
        };

        let mrr_chart = self
            .stats_service
            .total_mrr_chart(stats_service::MrrChartRequest {
                tenant_id,
                start_date,
                end_date,
                plans_id,
            })
            .await
            .map_err(|e| Status::internal(format!("Failed to fetch mrr chart: {}", e)))?;

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

    async fn mrr_breakdown(
        &self,
        request: Request<MrrBreakdownRequest>,
    ) -> Result<Response<MrrBreakdownResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();


        let mrr_breakdown = self
            .stats_service
            .mrr_breakdown(stats_service::MRRBreakdownRequest {
                tenant_id,
                scope: mapping::mrr_breakdown_scope_from_server(
                    MrrBreakdownScope::try_from(req.scope)
                        .map_err(|e| Status::invalid_argument(format!("Failed to parse scope: {}", e)))?
                ),
            })
            .await
            .map_err(|e| Status::internal(format!("Failed to fetch mrr breakdown: {}", e)))?;

        Ok(Response::new(MrrBreakdownResponse {
            mmr_breakdown: Some(mapping::mrr_breakdown_to_server(&mrr_breakdown)),
        }))
    }

    async fn mrr_log(
        &self,
        request: Request<MrrLogRequest>,
    ) -> Result<Response<MrrLogResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let mrr_log = self
            .stats_service
            .mrr_log(stats_service::MrrLogRequest {
                tenant_id,
                before: req.before,
                after: req.after,
            })
            .await
            .map_err(|e| Status::internal(format!("Failed to fetch mrr log: {}", e)))?;

        Ok(Response::new(MrrLogResponse {
            entries: mrr_log
                .entries
                .into_iter()
                .map(|entry| grpc::MrrLogEntry {
                    mrr_type: mapping::map_mrr_type(entry.mrr_type).into(),
                    customer_id: entry.customer_id,
                    customer_name: entry.customer_name,
                    applies_to: Some(shared::mapping::date::to_proto(entry.applies_to)),
                    created_at: Some(shared::mapping::datetime::offset_datetime_to_timestamp(entry.created_at)),
                    description: entry.description,
                    plan_name: entry.plan_name,
                    subscription_id: entry.subscription_id,
                })
                .collect(),
        }))
    }

    async fn signup_sparkline(
        &self,
        request: Request<SignupSparklineRequest>,
    ) -> Result<Response<SignupSparklineRequestResponse>, Status> {
        let tenant_id = request.tenant()?;

        let res = self
            .stats_service
            .signups_sparkline(tenant_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to fetch signup sparkline: {}", e)))?;

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

    async fn trial_conversion_rate_sparkline(
        &self,
        request: Request<TrialConversionRateSparklineRequest>,
    ) -> Result<Response<TrialConversionRateSparklineResponse>, Status> {
        let tenant_id = request.tenant()?;

        let res = self
            .stats_service
            .trial_conversion_rate_sparkline(tenant_id)
            .await
            .map_err(|e| {
                Status::internal(format!(
                    "Failed to fetch trial conversion rate sparkline: {}",
                    e
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

    async fn top_revenue_by_customer(
        &self,
        request: Request<TopRevenueByCustomerRequest>,
    ) -> Result<Response<TopRevenueByCustomerResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let top_revenue_by_customer = self
            .stats_service
            .top_revenue_by_customer(RevenueByCustomerRequest {
                tenant_id,
                limit: req.count,
            })
            .await
            .map_err(|e| {
                Status::internal(format!("Failed to fetch top revenue by customer: {}", e))
            })?;

        let top_revenue_by_customer = top_revenue_by_customer
            .into_iter()
            .map(|customer| grpc::RevenueByCustomer {
                customer_id: customer.customer_id.to_string(),
                customer_name: customer.customer_name,
                revenue: customer.revenue,
            })
            .collect();

        Ok(Response::new(TopRevenueByCustomerResponse {
            revenue_by_customer: top_revenue_by_customer,
        }))
    }

    //     #[tracing::instrument(skip_all)]
    //     async fn stats(&self, request: Request<StatsRequest>) -> Result<Response<StatsResponse>, Status> {
    //
    //
    //         todo!()
    //     }
    //
    //     #[tracing::instrument(skip_all)]
    //     async fn breakdown(&self, request: Request<BreakdownRequest>) -> Result<Response<BreakdownResponse>, Status> {
    //
    //         let req = request.into_inner();
    //
    //
    //
    //
    //         unimplemented!()
    //
    //         // Ok(Response::new(BreakdownResponse {
    //         //     currency_code: "".to_string(),
    //         //     start_date: req.start_date,
    //         //     end_date: req.end_date,
    //         //     total: 0,
    //         //     new_customer: None, // TODO paid ? free ? probably paid + another called "new_signups"
    //         //     new_subscription: None,
    //         //     expansions: None,
    //         //     contractions: None,
    //         //     churn: None,
    //         //     reactivation: None,
    //         //     new_trial: None,
    //         // }))
    //     }
    //
    //
    //     /*
    //     Type of stats :
    //     - trend (aka current_total + increase last period over previous period)
    //     - current_count
    //     - current_amount
    //     - current_count_with_amount
    //     - trend_with_data
    //
    //
    //      */
    //     async fn dashboard_stats(&self, request: Request<DashboardStatsRequest>) -> Result<Response<DashboardStatsResponse>, Status> {
    //
    //
    //         /*
    //         TODO currency management :
    //
    //         - we want all the data in the site's currency
    //         - therefore we want to convert other currencies
    //         - we can keep a daily updated exchange rate in the db
    //         - but what rate do we use ? the rate at the time of the transaction (but in this case we need to dat that in the trigger) ? or the rate at the time of the query ?
    //
    //
    // // => multicurrency is an ee feature
    //          */
    //
    //         let tenant = request.tenant()?;
    //
    //         let db_tenant = db::tenants::get_tenant_by_id()
    //             .bind(&self.get_connection().await?, &tenant)
    //             .one()
    //             .await
    //             .map_err(|e| {
    //                 tonic::Status::internal("Unable to get tenant.")
    //                     .set_source(Arc::new(e))
    //                     .clone()
    //             })?;
    //
    //         let currency = &db_tenant.currency; // TODO
    //
    //         let req = request.into_inner();
    //
    //         let connection = self.get_connection().await?;
    //
    //         // 1 - total net revenue + increase last 7 days over rpeious 7 days
    //         let revenue_trend = db::stats::query_revenue_trend()
    //             .params(
    //                 &connection,
    //                 &QueryRevenueTrendParams {
    //                     period_days: 7,
    //                     tenant_id: tenant.clone(),
    //                     currency,
    //                 }
    //             ).one()
    //             .await
    //             .map_err(|e| {
    //                 tonic::Status::internal("Failed to compute revenue trend.")
    //                     .set_source(Arc::new(e))
    //                     .clone()
    //             })?;
    //
    //         let pending_invoices = db::stats::query_pending_invoices()
    //             .bind(
    //                 &connection,
    //                 &tenant
    //             ).one()
    //             .await
    //             .map_err(|e| {
    //                 tonic::Status::internal("Failed to compute pending invoices.")
    //                     .set_source(Arc::new(e))
    //                     .clone()
    //             })?;
    //
    //
    //         let subscriptions = db::stats::query_subscription_trend()
    //             .bind(
    //                 &connection,
    //                 &tenant,
    //                 &7f64
    //             ).one()
    //             .await
    //             .map_err(|e| {
    //                 tonic::Status::internal("Failed to compute total subscriptions.")
    //                     .set_source(Arc::new(e))
    //                     .clone()
    //             })?;
    //
    //
    //
    //
    //
    //
    //
    //         // total subscribers
    //         // subscribers today
    //         // subscriber trend (7 days)
    //
    //         // total revenue
    //         // revenue today
    //         // revenue this month
    //         // revenue trend (7 days)
    //
    //
    //        Ok(Response::new(DashboardStatsResponse {
    //             total_net_revenue: None,
    //             total_active_subscriptions: None,
    //             pending_invoices: None,
    //             new_customers: None,
    //             trial_conversions: None,
    //             top_revenue_by_customer: None,
    //             mmr_movement_breakdown: None,
    //         }))
    //     }
}
