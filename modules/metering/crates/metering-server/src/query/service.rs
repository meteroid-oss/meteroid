use metering_grpc::meteroid::metering::v1::usage_query_service_server::UsageQueryService as UsageQueryServiceGrpc;
use rust_decimal::prelude::FromPrimitive;
use std::sync::Arc;

use common_grpc::meteroid::common::v1::Decimal;
use metering_grpc::meteroid::metering::v1::meter::AggregationType;
use metering_grpc::meteroid::metering::v1::query_meter_request::QueryWindowSize;
use metering_grpc::meteroid::metering::v1::query_meter_response as grpc;
use metering_grpc::meteroid::metering::v1::{
    DiscoverEventCodesRequest, DiscoverEventCodesResponse, DiscoverPropertiesRequest,
    DiscoverPropertiesResponse, DiscoverPropertyValuesRequest, DiscoverPropertyValuesResponse,
    DiscoveredProperty, Event, EventCodeCount, ListCustomerFirstEventsRequest,
    ListCustomerFirstEventsResponse, ListTenantFirstEventsRequest, ListTenantFirstEventsResponse,
    PollUsageActivityRequest, PollUsageActivityResponse, ProfileEventPropertiesRequest,
    ProfileEventPropertiesResponse, PropertyNumericStats, PropertyProfile, PropertyValueCount,
    PurgeTenantsRequest, PurgeTenantsResponse, QueryMeterRequest, QueryMeterResponse,
    QueryRawEventsRequest, QueryRawEventsResponse, list_customer_first_events_response,
    list_tenant_first_events_response, poll_usage_activity_response, property_filter,
    query_raw_events_request::SortOrder,
};
use tonic::{Request, Response, Status};

use common_domain::identifiers::{parse_timezone, validate_code};
use common_domain::ids::{CustomerId, TenantId};
use metering::connectors::Connector;
use metering::domain::{
    ActivityPosition, AttributionSegment, EventAttribution, EventClaim, EventCursor, EventMatch,
    EventSortOrder, OriginFilter, PropertyFilter, PropertyFilterOp, QueryMeterParams,
    QueryRawEventsParams, SegmentationFilter, TenantEventScan, UsageActivityParams, WindowSize,
};
use metering::error::MeteringApiError;
use metering::utils::{datetime_to_timestamp, rfc3339_to_datetime, timestamp_to_datetime};
use metric_core::domain::{TopNFold, TopNRow, apply_top_n_by_customer, apply_top_n_with};

const MAX_DISCOVERY_TEXT_LEN: usize = 1024;
const MAX_DISCOVERY_LOOKBACK_DAYS: u32 = 366;

enum TopNBreakdown {
    Customer,
    Property(String),
}

fn top_n_breakdown(params: &QueryMeterParams) -> Result<TopNBreakdown, Status> {
    if params.group_by_customer {
        return Ok(TopNBreakdown::Customer);
    }
    let mut dims: Vec<&String> = params.group_by.iter().collect();
    if let Some(segmentation) = params
        .segmentation_filter
        .as_ref()
        .filter(|_| !params.merge_segments)
    {
        match segmentation {
            SegmentationFilter::Independent(filters) => {
                dims.extend(filters.iter().map(|(dim, _)| dim))
            }
            SegmentationFilter::Linked {
                dimension1_key,
                dimension2_key,
                ..
            } => dims.extend([dimension1_key, dimension2_key]),
        }
    }
    dims.sort();
    dims.dedup();
    match dims.as_slice() {
        [dim] => Ok(TopNBreakdown::Property((*dim).clone())),
        _ => Err(Status::invalid_argument(
            "top_n needs group_by_customer or exactly one group-by property",
        )),
    }
}

/// `None` when the event code is empty: nothing to discover yet.
fn discovery_event_code(code: &str) -> Result<Option<&str>, Status> {
    if code.is_empty() {
        return Ok(None);
    }
    validate_code(code).map_err(|e| Status::invalid_argument(e.to_string()))?;
    Ok(Some(code))
}

fn validate_discovery_text(name: &str, value: &str) -> Result<(), Status> {
    if value.len() > MAX_DISCOVERY_TEXT_LEN {
        return Err(Status::invalid_argument(format!(
            "{name} exceeds {MAX_DISCOVERY_TEXT_LEN} bytes"
        )));
    }
    Ok(())
}

fn map_property_filters(
    filters: Vec<metering_grpc::meteroid::metering::v1::PropertyFilter>,
) -> Vec<PropertyFilter> {
    filters
        .into_iter()
        .map(|f| {
            let op = match f.op() {
                property_filter::FilterOp::In => PropertyFilterOp::In,
                property_filter::FilterOp::NotIn => PropertyFilterOp::NotIn,
            };
            PropertyFilter {
                property: f.property,
                op,
                values: f.values,
            }
        })
        .collect()
}

fn map_attribution(
    attribution: metering_grpc::meteroid::metering::v1::UsageAttribution,
) -> Result<EventAttribution, Status> {
    let non_empty = |property: &str, value: &str| {
        if property.is_empty() || value.is_empty() {
            Err(Status::invalid_argument(
                "attribution properties and values must not be empty",
            ))
        } else {
            Ok(())
        }
    };
    if attribution.segments.is_empty() {
        return Err(Status::invalid_argument(
            "an attribution needs at least one segment",
        ));
    }
    let segments = attribution
        .segments
        .into_iter()
        .map(|segment| {
            let matching = segment
                .matching
                .map(|matching| {
                    non_empty(&matching.property, &matching.value)?;
                    Ok::<_, Status>(EventMatch {
                        property: matching.property,
                        value: matching.value,
                    })
                })
                .transpose()?;
            let excluded = segment
                .excluded
                .into_iter()
                .map(|claim| {
                    non_empty(&claim.property, &claim.value)?;
                    let from = claim.from.map(timestamp_to_datetime).ok_or_else(|| {
                        Status::invalid_argument("an attribution claim needs `from`")
                    })?;
                    let to = claim.to.map(timestamp_to_datetime);
                    if to.is_some_and(|to| to <= from) {
                        return Err(Status::invalid_argument(
                            "an attribution claim must end after it starts",
                        ));
                    }
                    Ok(EventClaim {
                        property: claim.property,
                        value: claim.value,
                        from,
                        to,
                    })
                })
                .collect::<Result<Vec<_>, Status>>()?;
            Ok(AttributionSegment {
                from: segment.from.map(timestamp_to_datetime),
                to: segment.to.map(timestamp_to_datetime),
                matching,
                excluded,
            })
        })
        .collect::<Result<Vec<_>, Status>>()?;
    Ok(EventAttribution { segments })
}

fn to_grpc_usage(r: metric_core::domain::Usage, other: bool) -> grpc::Usage {
    grpc::Usage {
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
        other,
    }
}

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

        let window_time_zone = req
            .timezone
            .as_deref()
            .map(parse_timezone)
            .transpose()
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let aggregation_type: AggregationType = req.meter_aggregation_type();

        let meter_aggregation = aggregation_type.into();

        let window_size_grpc: QueryWindowSize = req.window_size();

        let window_size = match window_size_grpc {
            QueryWindowSize::Minute => Some(WindowSize::Minute),
            QueryWindowSize::Hour => Some(WindowSize::Hour),
            QueryWindowSize::Day => Some(WindowSize::Day),
            QueryWindowSize::AggregateAll => None,
        };

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

        let filters = map_property_filters(req.filters);
        let attribution = req.attribution.map(map_attribution).transpose()?;

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
            window_time_zone,
            segmentation_filter,
            merge_segments: req.merge_segments,
            filters,
            attribution,
            origin: OriginFilter::All,
            group_by_customer: req.group_by_customer,
            from: req
                .from
                .map(timestamp_to_datetime)
                .ok_or(Status::invalid_argument("from is required"))?,
            to: req.to.map(timestamp_to_datetime),
            keep_last_events: req.keep_last_events,
        };

        let top_n = match req.top_n.filter(|n| *n > 0) {
            Some(n) => Some((n as usize, top_n_breakdown(&meter)?)),
            None => None,
        };
        let fold = if meter.aggregation.is_additive() {
            TopNFold::Additive
        } else {
            TopNFold::NonAdditive
        };

        let results = self
            .connector
            .query_meter(meter)
            .await
            .map_err(Into::<MeteringApiError>::into)?;

        let usage = match top_n {
            Some((n, TopNBreakdown::Customer)) => apply_top_n_by_customer(results, n, fold),
            Some((n, TopNBreakdown::Property(dim))) => apply_top_n_with(results, &dim, n, fold),
            None => results
                .into_iter()
                .map(|usage| TopNRow {
                    usage,
                    other: false,
                })
                .collect(),
        }
        .into_iter()
        .map(|row| to_grpc_usage(row.usage, row.other))
        .collect();

        Ok(Response::new(QueryMeterResponse {
            usage,
            attribution_applied: true,
            grouping_applied: true,
        }))
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
            limit: req.limit.min(1000),
            offset: req.offset,
            search: req.search,
            event_codes: req.event_codes,
            customer_ids: req
                .customer_ids
                .into_iter()
                .map(CustomerId::from_proto)
                .collect::<Result<Vec<_>, _>>()?,
            filters: map_property_filters(req.filters),
            sort_order,
            after: match (req.after_timestamp_nanos, req.after_id) {
                (Some(timestamp_nanos), Some(id)) => Some(EventCursor {
                    timestamp_nanos,
                    id,
                }),
                (None, None) => None,
                _ => {
                    return Err(Status::invalid_argument(
                        "after_timestamp_nanos and after_id must be set together",
                    ));
                }
            },
            keep_last_events: req.keep_last_events,
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

    #[tracing::instrument(skip_all)]
    async fn purge_tenants(
        &self,
        request: Request<PurgeTenantsRequest>,
    ) -> Result<Response<PurgeTenantsResponse>, Status> {
        let req = request.into_inner();
        let tenant_ids = req
            .tenant_ids
            .into_iter()
            .map(TenantId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        self.connector
            .purge_tenants(&tenant_ids)
            .await
            .map_err(Into::<MeteringApiError>::into)?;

        Ok(Response::new(PurgeTenantsResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn list_tenant_first_events(
        &self,
        request: Request<ListTenantFirstEventsRequest>,
    ) -> Result<Response<ListTenantFirstEventsResponse>, Status> {
        let req = request.into_inner();
        let tenants = req
            .tenants
            .into_iter()
            .map(|t| {
                Ok(TenantEventScan {
                    tenant_id: TenantId::from_proto(t.tenant_id)?,
                    from: t
                        .from
                        .map(|at| {
                            rfc3339_to_datetime(&at).map_err(|e| {
                                Status::invalid_argument(format!("Invalid `from`: {e}"))
                            })
                        })
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>, Status>>()?;

        let results = self
            .connector
            .list_tenant_first_events(&tenants)
            .await
            .map_err(Into::<MeteringApiError>::into)?;

        let tenants = results
            .into_iter()
            .map(|t| list_tenant_first_events_response::TenantFirstEvent {
                tenant_id: t.tenant_id.as_proto(),
                first_ingested_at: Some(datetime_to_timestamp(t.first_ingested_at)),
            })
            .collect();

        Ok(Response::new(ListTenantFirstEventsResponse { tenants }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_customer_first_events(
        &self,
        request: Request<ListCustomerFirstEventsRequest>,
    ) -> Result<Response<ListCustomerFirstEventsResponse>, Status> {
        let req = request.into_inner();
        let tenant_id = TenantId::from_proto(req.tenant_id)?;
        let customer_ids = req
            .customer_ids
            .into_iter()
            .map(CustomerId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        let results = self
            .connector
            .list_customer_first_events(&tenant_id, &customer_ids)
            .await
            .map_err(Into::<MeteringApiError>::into)?;

        let customers = results
            .into_iter()
            .map(
                |c| list_customer_first_events_response::CustomerFirstEvent {
                    customer_id: c.customer_id.as_proto(),
                    first_ingested_at: c.first_ingested_at.to_rfc3339(),
                },
            )
            .collect();

        Ok(Response::new(ListCustomerFirstEventsResponse { customers }))
    }

    #[tracing::instrument(skip_all)]
    async fn poll_usage_activity(
        &self,
        request: Request<PollUsageActivityRequest>,
    ) -> Result<Response<PollUsageActivityResponse>, Status> {
        let req = request.into_inner();
        if req.shard_count == 0 || req.shard_index >= req.shard_count {
            return Err(Status::invalid_argument("shard_index out of range"));
        }

        let params = UsageActivityParams {
            since: req
                .since
                .map(timestamp_to_datetime)
                .ok_or(Status::invalid_argument("since is required"))?,
            after: req
                .after
                .map(|after| {
                    Ok::<_, Status>(ActivityPosition {
                        last_seen_at: after
                            .last_seen_at
                            .map(timestamp_to_datetime)
                            .ok_or(Status::invalid_argument("after.last_seen_at is required"))?,
                        tenant_id: TenantId::from_proto(after.tenant_id)?,
                        customer_id: CustomerId::from_proto(after.customer_id)?,
                    })
                })
                .transpose()?,
            shard_index: req.shard_index,
            shard_count: req.shard_count,
            limit: req.limit.clamp(1, 10_000),
        };

        let results = self
            .connector
            .poll_usage_activity(params)
            .await
            .map_err(Into::<MeteringApiError>::into)?;

        let customers = results
            .into_iter()
            .map(|c| poll_usage_activity_response::CustomerActivity {
                tenant_id: c.tenant_id.as_proto(),
                customer_id: c.customer_id.as_proto(),
                last_seen_at: Some(datetime_to_timestamp(c.last_seen_at)),
                min_event_at: Some(datetime_to_timestamp(c.min_event_at)),
            })
            .collect();

        Ok(Response::new(PollUsageActivityResponse { customers }))
    }

    #[tracing::instrument(skip_all)]
    async fn discover_event_codes(
        &self,
        request: Request<DiscoverEventCodesRequest>,
    ) -> Result<Response<DiscoverEventCodesResponse>, Status> {
        let req = request.into_inner();
        let tenant_id = TenantId::from_proto(req.tenant_id)?;
        validate_discovery_text("search", &req.search)?;
        let limit = match req.limit {
            0 => 50,
            n => n.min(200),
        };

        let rows = self
            .connector
            .discover_event_codes(&tenant_id, &req.search, limit)
            .await
            .map_err(Into::<MeteringApiError>::into)?;

        let codes = rows
            .into_iter()
            .map(|c| EventCodeCount {
                code: c.code,
                count: c.count,
            })
            .collect();

        Ok(Response::new(DiscoverEventCodesResponse { codes }))
    }

    #[tracing::instrument(skip_all)]
    async fn discover_properties(
        &self,
        request: Request<DiscoverPropertiesRequest>,
    ) -> Result<Response<DiscoverPropertiesResponse>, Status> {
        let req = request.into_inner();
        let tenant_id = TenantId::from_proto(req.tenant_id)?;
        let Some(event_code) = discovery_event_code(&req.event_code)? else {
            return Ok(Response::new(DiscoverPropertiesResponse::default()));
        };
        let lookback_days = match req.lookback_days {
            0 => 90,
            n => n.min(MAX_DISCOVERY_LOOKBACK_DAYS),
        };
        let limit = match req.limit {
            0 => 100,
            n => n.min(500),
        };

        let rows = self
            .connector
            .discover_properties(&tenant_id, event_code, lookback_days, limit)
            .await
            .map_err(Into::<MeteringApiError>::into)?;

        let properties = rows
            .into_iter()
            .map(|p| DiscoveredProperty {
                key: p.key,
                cardinality: p.cardinality,
                sample_values: p.sample_values,
                numeric_ratio: p.numeric_ratio,
            })
            .collect();

        Ok(Response::new(DiscoverPropertiesResponse { properties }))
    }

    #[tracing::instrument(skip_all)]
    async fn discover_property_values(
        &self,
        request: Request<DiscoverPropertyValuesRequest>,
    ) -> Result<Response<DiscoverPropertyValuesResponse>, Status> {
        let req = request.into_inner();
        let tenant_id = TenantId::from_proto(req.tenant_id)?;
        validate_discovery_text("property_key", &req.property_key)?;
        validate_discovery_text("search", &req.search)?;
        let Some(event_code) = discovery_event_code(&req.event_code)? else {
            return Ok(Response::new(DiscoverPropertyValuesResponse::default()));
        };
        let limit = match req.limit {
            0 => 20,
            n => n.min(100),
        };

        let rows = self
            .connector
            .discover_property_values(
                &tenant_id,
                event_code,
                &req.property_key,
                &req.search,
                limit,
            )
            .await
            .map_err(Into::<MeteringApiError>::into)?;

        let values = rows
            .into_iter()
            .map(|v| PropertyValueCount {
                value: v.value,
                count: v.count,
            })
            .collect();

        Ok(Response::new(DiscoverPropertyValuesResponse { values }))
    }

    #[tracing::instrument(skip_all)]
    async fn profile_event_properties(
        &self,
        request: Request<ProfileEventPropertiesRequest>,
    ) -> Result<Response<ProfileEventPropertiesResponse>, Status> {
        let req = request.into_inner();
        let tenant_id = TenantId::from_proto(req.tenant_id)?;
        let Some(event_code) = discovery_event_code(&req.event_code)? else {
            return Ok(Response::new(ProfileEventPropertiesResponse::default()));
        };
        if req.from_ts < 0 || req.to_ts < 0 {
            return Err(Status::invalid_argument(
                "from_ts and to_ts must not be negative",
            ));
        }
        if req.from_ts > 0 && req.to_ts > 0 && req.to_ts <= req.from_ts {
            return Err(Status::invalid_argument("to_ts must be after from_ts"));
        }
        let top_values = match req.top_values {
            0 => 8,
            n => n.min(50),
        };
        let limit = match req.limit {
            0 => 100,
            n => n.min(500),
        };

        let profile = self
            .connector
            .profile_event_properties(
                &tenant_id,
                event_code,
                req.from_ts,
                req.to_ts,
                top_values,
                limit,
            )
            .await
            .map_err(Into::<MeteringApiError>::into)?;

        let properties = profile
            .properties
            .into_iter()
            .map(|p| PropertyProfile {
                key: p.key,
                event_count: p.event_count,
                coverage_ratio: p.coverage_ratio,
                cardinality: p.cardinality,
                numeric_ratio: p.numeric_ratio,
                numeric: p.numeric.map(|n| PropertyNumericStats {
                    min: n.min,
                    max: n.max,
                    avg: n.avg,
                    sum: n.sum,
                    p50: n.p50,
                    p95: n.p95,
                }),
                top_values: p
                    .top_values
                    .into_iter()
                    .map(|v| PropertyValueCount {
                        value: v.value,
                        count: v.count,
                    })
                    .collect(),
            })
            .collect();

        Ok(Response::new(ProfileEventPropertiesResponse {
            total_events: profile.total_events,
            properties,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metering::domain::MeterAggregation;

    fn params(group_by: &[&str], segmentation: Option<SegmentationFilter>) -> QueryMeterParams {
        QueryMeterParams {
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "api_call".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: segmentation,
            merge_segments: false,
            filters: vec![],
            attribution: None,
            origin: OriginFilter::All,
            group_by_customer: false,
            group_by: group_by.iter().map(|s| s.to_string()).collect(),
            window_size: None,
            window_time_zone: None,
            from: rfc3339_to_datetime("2026-01-01T00:00:00Z").unwrap(),
            to: None,
            keep_last_events: false,
        }
    }

    fn independent(dims: &[&str]) -> Option<SegmentationFilter> {
        Some(SegmentationFilter::Independent(
            dims.iter()
                .map(|d| (d.to_string(), vec!["x".to_string()]))
                .collect(),
        ))
    }

    #[test]
    fn top_n_ranks_customers_when_grouping_by_customer() {
        let mut p = params(&["region", "model"], None);
        p.group_by_customer = true;
        assert!(matches!(top_n_breakdown(&p), Ok(TopNBreakdown::Customer)));
    }

    #[test]
    fn top_n_ranks_the_single_breakdown_dimension() {
        let breakdown = |p| match top_n_breakdown(&p) {
            Ok(TopNBreakdown::Property(dim)) => dim,
            _ => panic!("expected a property breakdown"),
        };
        assert_eq!(breakdown(params(&["region"], None)), "region");
        assert_eq!(breakdown(params(&[], independent(&["tier"]))), "tier");
        assert_eq!(breakdown(params(&["tier"], independent(&["tier"]))), "tier");
        let mut merged = params(&["region"], independent(&["tier"]));
        merged.merge_segments = true;
        assert_eq!(breakdown(merged), "region");
    }

    #[test]
    fn top_n_rejects_zero_or_several_dimensions() {
        for p in [
            params(&[], None),
            params(&["region", "model"], None),
            params(&["region"], independent(&["tier"])),
        ] {
            let err = top_n_breakdown(&p).err().expect("rejected");
            assert_eq!(err.code(), tonic::Code::InvalidArgument);
        }
    }
}
