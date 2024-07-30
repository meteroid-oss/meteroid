use metering_grpc::meteroid::metering::v1::usage_query_service_server::UsageQueryService as UsageQueryServiceGrpc;
use rust_decimal::prelude::FromPrimitive;
use std::sync::Arc;

use common_grpc::meteroid::common::v1::Decimal;
use metering_grpc::meteroid::metering::v1::meter::AggregationType;
use metering_grpc::meteroid::metering::v1::query_meter_request::QueryWindowSize;
use metering_grpc::meteroid::metering::v1::query_meter_response as grpc;
use metering_grpc::meteroid::metering::v1::{
    QueryMeterRequest, QueryMeterResponse, QueryRawEventsRequest, QueryRawEventsResponse,
};
use tonic::{Request, Response, Status};

use crate::connectors::Connector;
use crate::domain::{Customer, QueryMeterParams, WindowSize};
use crate::utils::{datetime_to_timestamp, timestamp_to_datetime};

#[derive(Clone)]
pub struct UsageQueryService {
    pub connector: Arc<dyn Connector + Send + Sync>,
}

impl UsageQueryService {
    pub fn new(connector: Arc<dyn Connector + Send + Sync>) -> Self {
        UsageQueryService { connector }
    }
}

#[tonic::async_trait]
impl UsageQueryServiceGrpc for UsageQueryService {
    #[tracing::instrument(skip_all)]
    async fn query_meter(
        &self,
        request: Request<QueryMeterRequest>,
    ) -> Result<Response<QueryMeterResponse>, Status> {
        let req = request.into_inner();

        let aggregation_type: AggregationType = req
            .meter_aggregation_type
            .try_into()
            .map_err(|_| Status::internal("unknown aggregation_type"))?;

        let meter_aggregation = aggregation_type.into();

        let window_size_grpc: QueryWindowSize = req
            .window_size
            .try_into()
            .map_err(|_| Status::invalid_argument("unknown window_size"))?;

        let window_size = match window_size_grpc {
            QueryWindowSize::Minute => Some(WindowSize::Minute),
            QueryWindowSize::Hour => Some(WindowSize::Hour),
            QueryWindowSize::Day => Some(WindowSize::Day),
            QueryWindowSize::AggregateAll => None,
        };

        let meter = QueryMeterParams {
            aggregation: meter_aggregation,
            namespace: req.tenant_id,
            meter_slug: req.meter_slug,
            event_name: req.event_name,
            customers: req
                .customers
                .iter()
                .map(|c| Customer {
                    id: c.meteroid_id.clone(),
                    external_id: c.external_id.clone(),
                })
                .collect(),
            group_by: req.group_by_properties,
            window_size,
            window_time_zone: req.timezone,
            filter_group_by: req
                .filter_properties
                .iter()
                .map(|filter| (filter.property_name.clone(), filter.property_value.clone()))
                .collect(),
            from: req
                .from
                .map(timestamp_to_datetime)
                .ok_or(Status::invalid_argument("from is required"))?,
            to: req.to.map(timestamp_to_datetime),
        };

        let results =
            self.connector.query_meter(meter).await.map_err(|e| {
                Status::internal(format!("Failed to query meter : {}", e.to_string()))
            })?;

        let usage = results
            .into_iter()
            .map(|r| grpc::Usage {
                window_start: Some(datetime_to_timestamp(r.window_start)),
                window_end: Some(datetime_to_timestamp(r.window_end)),
                value: rust_decimal::Decimal::from_f64(r.value).map(|v| Decimal {
                    value: v.to_string(),
                }),
                customer_id: r.customer_id,
                dimensions: r
                    .group_by
                    .into_iter()
                    .map(|(k, v)| (k, grpc::usage::DimensionValueField { value: v }))
                    .collect(),
            })
            .collect();

        Ok(Response::new(QueryMeterResponse { usage }))
    }

    #[tracing::instrument(skip_all)]
    async fn query_raw_events(
        &self,
        _request: Request<QueryRawEventsRequest>,
    ) -> Result<Response<QueryRawEventsResponse>, Status> {
        todo!()
    }
}
