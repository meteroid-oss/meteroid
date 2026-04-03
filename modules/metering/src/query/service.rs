use metering_grpc::meteroid::metering::v1::usage_query_service_server::UsageQueryService as UsageQueryServiceGrpc;
use rust_decimal::prelude::FromPrimitive;
use std::sync::Arc;

use common_grpc::meteroid::common::v1::Decimal;
use metering_grpc::meteroid::metering::v1::meter::AggregationType;
use metering_grpc::meteroid::metering::v1::query_meter_request::QueryWindowSize;
use metering_grpc::meteroid::metering::v1::query_meter_response as grpc;
use metering_grpc::meteroid::metering::v1::{
    QueryMeterRequest, QueryMeterResponse, QueryRawEventsRequest, QueryRawEventsResponse,
    query_raw_events_request::SortOrder,
};
use tonic::{Request, Response, Status};

use crate::connectors::Connector;
use crate::domain::{
    EventSortOrder, QueryMeterParams, QueryRawEventsParams, SegmentationFilter, WindowSize,
};
use crate::error::MeteringApiError;
use crate::utils::{datetime_to_timestamp, timestamp_to_datetime};
use common_domain::identifiers::{validate_code, validate_property_key, validate_timezone};
use common_domain::ids::{CustomerId, TenantId};
use metering_grpc::meteroid::metering::v1::Event;

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

        validate_code(&req.code).map_err(|e| Status::invalid_argument(e.to_string()))?;

        if let Some(ref key) = req.value_property {
            validate_property_key(key).map_err(|e| Status::invalid_argument(e.to_string()))?;
        }

        for key in &req.group_by_properties {
            validate_property_key(key).map_err(|e| Status::invalid_argument(e.to_string()))?;
        }

        if let Some(ref tz) = req.timezone {
            validate_timezone(tz).map_err(|e| Status::invalid_argument(e.to_string()))?;
        }

        if let Some(ref sf) = req.segmentation_filter {
            use metering_grpc::meteroid::metering::v1::segmentation_filter::Filter;
            match &sf.filter {
                Some(Filter::Independent(ind)) => {
                    for f in &ind.filters {
                        validate_property_key(&f.property_name)
                            .map_err(|e| Status::invalid_argument(e.to_string()))?;
                    }
                }
                Some(Filter::Linked(linked)) => {
                    validate_property_key(&linked.dimension1_key)
                        .map_err(|e| Status::invalid_argument(e.to_string()))?;
                    validate_property_key(&linked.dimension2_key)
                        .map_err(|e| Status::invalid_argument(e.to_string()))?;
                }
                None => {}
            }
        }

        let aggregation_type: AggregationType = req.meter_aggregation_type();

        let meter_aggregation = aggregation_type.into();

        let window_size_grpc: QueryWindowSize = req.window_size();

        let window_size = match window_size_grpc {
            QueryWindowSize::Minute => Some(WindowSize::Minute),
            QueryWindowSize::Hour => Some(WindowSize::Hour),
            QueryWindowSize::Day => Some(WindowSize::Day),
            QueryWindowSize::AggregateAll => None,
        };

        // Convert proto segmentation filter to domain segmentation filter
        let segmentation_filter = if let Some(sf) = req.segmentation_filter {
            match sf.filter {
                Some(
                    metering_grpc::meteroid::metering::v1::segmentation_filter::Filter::Independent(
                        ind,
                    ),
                ) => {
                    let filters = ind
                        .filters
                        .into_iter()
                        .map(|f| (f.property_name, f.property_value))
                        .collect();
                    Some(SegmentationFilter::Independent(filters))
                }
                Some(
                    metering_grpc::meteroid::metering::v1::segmentation_filter::Filter::Linked(
                        linked,
                    ),
                ) => {
                    let values = linked
                        .linked_values
                        .into_iter()
                        .map(|(k, v)| (k, v.values))
                        .collect();
                    Some(SegmentationFilter::Linked {
                        dimension1_key: linked.dimension1_key,
                        dimension2_key: linked.dimension2_key,
                        values,
                    })
                }
                None => None,
            }
        } else {
            None
        };

        let meter = QueryMeterParams {
            aggregation: meter_aggregation,
            tenant_id: TenantId::from_proto(req.tenant_id)?,
            code: req.code,
            value_property: req.value_property,
            customer_ids: req
                .customer_ids
                .into_iter()
                .map(CustomerId::from_proto)
                .collect::<Result<Vec<_>, _>>()?,
            group_by: req.group_by_properties,
            window_size,
            window_time_zone: req.timezone,
            segmentation_filter,
            from: req
                .from
                .map(timestamp_to_datetime)
                .ok_or(Status::invalid_argument("from is required"))?,
            to: req.to.map(timestamp_to_datetime),
        };

        let results = self
            .connector
            .query_meter(meter)
            .await
            .map_err(Into::<MeteringApiError>::into)?;

        let usage = results
            .into_iter()
            .map(|r| grpc::Usage {
                window_start: Some(datetime_to_timestamp(r.window_start)),
                window_end: Some(datetime_to_timestamp(r.window_end)),
                value: rust_decimal::Decimal::from_f64(r.value).map(|v| Decimal {
                    value: v.to_string(),
                }),
                customer_id: r.customer_id.map(|id| id.as_proto()),
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
        request: Request<QueryRawEventsRequest>,
    ) -> Result<Response<QueryRawEventsResponse>, Status> {
        let req = request.into_inner();

        for code in &req.event_codes {
            validate_code(code).map_err(|e| Status::invalid_argument(e.to_string()))?;
        }

        let sort_order = match req.sort_order() {
            SortOrder::TimestampDesc => EventSortOrder::TimestampDesc,
            SortOrder::TimestampAsc => EventSortOrder::TimestampAsc,
            SortOrder::IngestedDesc => EventSortOrder::IngestedDesc,
            SortOrder::IngestedAsc => EventSortOrder::IngestedAsc,
        };

        let params = QueryRawEventsParams {
            tenant_id: TenantId::from_proto(req.tenant_id)?,
            from: req
                .from
                .map(timestamp_to_datetime)
                .ok_or(Status::invalid_argument("from is required"))?,
            to: req.to.map(timestamp_to_datetime),
            limit: req.limit.min(1000), // Cap at 1000
            offset: req.offset,
            search: req.search,
            event_codes: req.event_codes,
            customer_ids: req
                .customer_ids
                .into_iter()
                .map(CustomerId::from_proto)
                .collect::<Result<Vec<_>, _>>()?,
            sort_order,
        };

        let result = self
            .connector
            .query_raw_events(params)
            .await
            .map_err(Into::<MeteringApiError>::into)?;

        let events = result
            .events
            .into_iter()
            .map(|raw_event| Event {
                id: raw_event.id,
                code: raw_event.code,
                customer_id: Some(
                    metering_grpc::meteroid::metering::v1::event::CustomerId::MeteroidCustomerId(
                        raw_event.customer_id.as_proto(),
                    ),
                ),
                timestamp: raw_event.timestamp.and_utc().to_rfc3339(),
                properties: raw_event.properties,
            })
            .collect();

        Ok(Response::new(QueryRawEventsResponse { events }))
    }
}
