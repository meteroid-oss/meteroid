use chrono::{NaiveDateTime, Timelike};
use common_domain::ids::{CustomerId, TenantId};
use common_grpc::middleware::client::LayeredClientService;

use crate::conversions::ProtoConv;
use error_stack::{Report, ResultExt, bail};
use metering_grpc::meteroid::metering::v1::internal_events_service_client::InternalEventsServiceClient;
use metering_grpc::meteroid::metering::v1::meter::AggregationType;
use metering_grpc::meteroid::metering::v1::query_meter_request::{self, QueryWindowSize};
use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;
use metering_grpc::meteroid::metering::v1::{
    DiscoverEventCodesRequest, DiscoverPropertiesRequest, DiscoverPropertyValuesRequest, Filter,
    InternalIngestRequest, ListCustomerFirstEventsRequest, ListTenantFirstEventsRequest,
    PollUsageActivityRequest, ProfileEventPropertiesRequest, PropertyFilter,
    PurgeTenantsRequest as MeteringPurgeTenantsRequest, QueryMeterRequest, QueryMeterResponse,
    QueryRawEventsRequest, SegmentationFilter, UsageAttribution, poll_usage_activity_request,
    property_filter,
    query_raw_events_request::SortOrder,
    segmentation_filter,
    segmentation_filter::{
        IndependentFilters, LinkedFilters, linked_filters::LinkedDimensionValues,
    },
    usage_attribution,
};
use meteroid_store::clients::pending_usage::{
    CustomerUsage, OriginFilter, PendingUsageRequest, PendingUsageSource,
};
use meteroid_store::clients::usage::{
    ActivityPosition, CustomerActivity, CustomerFirstIngestedEvent, DiscoveredProperty,
    EventCodeCount, EventPropertiesProfile, EventSearchOptions, EventSearchResult,
    GroupedUsageData, MetricEvent, MetricEventCursor, MetricUsageBreakdown, MetricUsagePoint,
    MetricUsageQuery, PropertyNumericStats, PropertyProfile, PropertyValueCount, TenantEventScan,
    TenantFirstIngestedEvent, TenantUsageGrain, TenantUsagePoint, UsageClient, UsageData,
    UsageSeriesPoint, WindowedUsageData, WindowedUsagePoint,
};
use meteroid_store::domain::{BillableMetric, UsagePeriod, UsageScope};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::tenants::KeepLastEventsLookup;
use meteroid_store::{StoreResult, domain};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::collections::HashMap;
use std::sync::{Arc, Once, OnceLock};
use std::time::Duration;

const GRPC_TIMEOUT: Duration = Duration::from_secs(10);

fn map_aggregation_type(agg: &domain::enums::BillingMetricAggregateEnum) -> i32 {
    (match agg {
        domain::enums::BillingMetricAggregateEnum::Count => AggregationType::Count,
        domain::enums::BillingMetricAggregateEnum::Latest => AggregationType::Latest,
        domain::enums::BillingMetricAggregateEnum::Max => AggregationType::Max,
        domain::enums::BillingMetricAggregateEnum::Min => AggregationType::Min,
        domain::enums::BillingMetricAggregateEnum::Mean => AggregationType::Mean,
        domain::enums::BillingMetricAggregateEnum::Sum => AggregationType::Sum,
        domain::enums::BillingMetricAggregateEnum::CountDistinct => AggregationType::CountDistinct,
    }) as i32
}

fn build_segmentation_filter(
    matrix: Option<domain::SegmentationMatrix>,
) -> Option<SegmentationFilter> {
    match matrix {
        Some(domain::SegmentationMatrix::Single(domain::Dimension { key, values, .. })) => {
            Some(SegmentationFilter {
                filter: Some(segmentation_filter::Filter::Independent(
                    IndependentFilters {
                        filters: vec![Filter {
                            property_name: key,
                            property_value: values,
                        }],
                    },
                )),
            })
        }
        Some(domain::SegmentationMatrix::Double {
            dimension1,
            dimension2,
        }) => Some(SegmentationFilter {
            filter: Some(segmentation_filter::Filter::Independent(
                IndependentFilters {
                    filters: vec![
                        Filter {
                            property_name: dimension1.key,
                            property_value: dimension1.values,
                        },
                        Filter {
                            property_name: dimension2.key,
                            property_value: dimension2.values,
                        },
                    ],
                },
            )),
        }),
        Some(domain::SegmentationMatrix::Linked {
            dimension1_key,
            dimension2_key,
            values,
        }) => {
            let linked_values = values
                .into_iter()
                .map(|(k, v)| (k, LinkedDimensionValues { values: v }))
                .collect();

            Some(SegmentationFilter {
                filter: Some(segmentation_filter::Filter::Linked(LinkedFilters {
                    dimension1_key,
                    dimension2_key,
                    linked_values,
                })),
            })
        }
        None => None,
    }
}

/// Translate a metric's domain filters into metering-proto `PropertyFilter`s for the rating
/// query. The four UI operators collapse to the engine's two: Equal/In → IN, NotEqual/NotIn
/// → NOT_IN (the metering compiler applies presence-required negation). No filters → empty
/// vec, which the metering query builder treats as a no-op.
fn build_property_filters(
    filters: Option<&domain::billable_metrics::MetricFilters>,
) -> Vec<PropertyFilter> {
    use domain::billable_metrics::MetricFilterOp;
    let Some(domain::billable_metrics::MetricFilters(list)) = filters else {
        return Vec::new();
    };
    list.iter()
        .map(|f| {
            let op = match f.op {
                MetricFilterOp::Equal | MetricFilterOp::In => property_filter::FilterOp::In,
                MetricFilterOp::NotEqual | MetricFilterOp::NotIn => {
                    property_filter::FilterOp::NotIn
                }
            };
            PropertyFilter {
                property: f.property.clone(),
                op: op as i32,
                values: f.values.clone(),
            }
        })
        .collect()
}

fn build_attribution(scope: &UsageScope) -> Option<UsageAttribution> {
    if scope.is_customer_wide() {
        return None;
    }
    Some(UsageAttribution {
        segments: scope
            .segments
            .iter()
            .map(|segment| usage_attribution::Segment {
                from: segment.from.map(datetime_to_timestamp),
                to: segment.until.map(datetime_to_timestamp),
                matching: segment
                    .matching
                    .as_ref()
                    .map(|filter| usage_attribution::EventMatch {
                        property: filter.property.clone(),
                        value: filter.value.clone(),
                    }),
                excluded: segment
                    .excluded
                    .iter()
                    .map(|claim| usage_attribution::EventClaim {
                        property: claim.filter.property.clone(),
                        value: claim.filter.value.clone(),
                        from: Some(datetime_to_timestamp(claim.from)),
                        to: claim.until.map(datetime_to_timestamp),
                    })
                    .collect(),
            })
            .collect(),
    })
}

/// A metering service predating usage filters ignores them and answers every event of the
/// customer: such an answer is refused rather than billed.
fn ensure_attributed(attributed: bool, response: &QueryMeterResponse) -> StoreResult<()> {
    if attributed && !response.attribution_applied {
        return Err(Report::new(StoreError::MeteringNotReady)
            .attach("the metering service does not apply usage filters yet"));
    }
    Ok(())
}

/// Reads that cannot honour a subscription's usage filter refuse a metric scoped by one.
fn ensure_customer_wide(metric: &BillableMetric, read: &str) -> StoreResult<()> {
    if !metric.usage_scope.is_customer_wide() {
        bail!(StoreError::InvalidArgument(format!(
            "{read} cannot apply a subscription's usage filter"
        )));
    }
    Ok(())
}

/// A metering service predating event discovery answers `Unimplemented`, reported as not ready so
/// callers can tell a rollout from an empty answer.
fn discovery_response<T>(
    rpc: &str,
    answer: Result<Result<tonic::Response<T>, tonic::Status>, tokio::time::error::Elapsed>,
) -> StoreResult<T> {
    match answer {
        Ok(Ok(response)) => Ok(response.into_inner()),
        Ok(Err(status)) if status.code() == tonic::Code::Unimplemented => {
            Err(Report::new(StoreError::MeteringNotReady).attach(format!(
                "{rpc}: the metering service does not serve event discovery yet"
            )))
        }
        Ok(Err(status)) => Err(Report::new(status)
            .change_context(StoreError::MeteringServiceError)
            .attach(format!("{rpc} failed"))),
        Err(_) => {
            Err(Report::new(StoreError::MeteringServiceError).attach(format!("{rpc} timed out")))
        }
    }
}

fn meter_request(
    tenant_id: &TenantId,
    keep_last_events: bool,
    customer_id: &CustomerId,
    metric: &BillableMetric,
    group_by: Option<&str>,
    window_size: QueryWindowSize,
    period: &UsagePeriod,
) -> QueryMeterRequest {
    QueryMeterRequest {
        tenant_id: tenant_id.as_proto(),
        keep_last_events,
        meter_slug: metric.id.to_string(),
        code: metric.code.clone(),
        meter_aggregation_type: map_aggregation_type(&metric.aggregation_type),
        customer_ids: vec![customer_id.to_string()],
        from: Some(datetime_to_timestamp(period.start)),
        to: Some(datetime_to_timestamp(period.end)),
        group_by_properties: metric
            .usage_group_key
            .iter()
            .map(String::as_str)
            .chain(group_by)
            .fold(Vec::<String>::new(), |mut names, name| {
                if !names.iter().any(|known| known == name) {
                    names.push(name.to_owned());
                }
                names
            }),
        window_size: window_size.into(),
        timezone: None,
        segmentation_filter: build_segmentation_filter(metric.segmentation_matrix.clone()),
        filters: build_property_filters(metric.filters.as_ref()),
        value_property: metric.aggregation_key.clone(),
        origin_filter: query_meter_request::OriginFilter::All.into(),
        merge_segments: false,
        attribution: build_attribution(&metric.usage_scope),
        ..Default::default()
    }
}

fn metric_usage_request(
    tenant_id: &TenantId,
    keep_last_events: bool,
    query: &MetricUsageQuery<'_>,
) -> StoreResult<QueryMeterRequest> {
    if query.period.start >= query.period.end {
        bail!(StoreError::InvalidArgument("invalid period".to_string()));
    }
    // Metering reads each listed customer apart; only a customer breakdown keeps them apart.
    if query.customer_ids.len() > 1 && query.breakdown != MetricUsageBreakdown::Customer {
        bail!(StoreError::InvalidArgument(
            "several customers need a customer breakdown".to_string()
        ));
    }
    let metric = query.metric;
    Ok(QueryMeterRequest {
        tenant_id: tenant_id.as_proto(),
        keep_last_events,
        code: metric.code.clone(),
        meter_aggregation_type: map_aggregation_type(&metric.aggregation_type),
        customer_ids: query.customer_ids.iter().map(|id| id.as_proto()).collect(),
        from: Some(datetime_to_timestamp(query.period.start)),
        to: Some(datetime_to_timestamp(query.period.end)),
        group_by_properties: match &query.breakdown {
            MetricUsageBreakdown::Property(property) => vec![property.clone()],
            MetricUsageBreakdown::None | MetricUsageBreakdown::Customer => vec![],
        },
        window_size: match query.grain {
            Some(TenantUsageGrain::Hour) => QueryWindowSize::Hour,
            Some(TenantUsageGrain::Day) => QueryWindowSize::Day,
            None => QueryWindowSize::AggregateAll,
        }
        .into(),
        timezone: None,
        segmentation_filter: build_segmentation_filter(
            metric.complete_segmentation_matrix().cloned(),
        ),
        merge_segments: true,
        filters: build_property_filters(metric.filters.as_ref()),
        value_property: metric.aggregation_key.clone(),
        origin_filter: query_meter_request::OriginFilter::All.into(),
        group_by_customer: query.breakdown == MetricUsageBreakdown::Customer,
        top_n: query.top_n.filter(|n| *n > 0),
        ..Default::default()
    })
}

fn metric_usage_points(
    query: &MetricUsageQuery<'_>,
    response: QueryMeterResponse,
) -> StoreResult<Vec<MetricUsagePoint>> {
    // A metering service predating customer grouping answers one row across all customers.
    let grouped = query.breakdown == MetricUsageBreakdown::Customer || query.top_n.is_some();
    if grouped && !response.grouping_applied {
        return Err(Report::new(StoreError::MeteringNotReady)
            .attach("the metering service does not break usage down by customer yet"));
    }

    let mut points = Vec::with_capacity(response.usage.len());
    for usage in response.usage {
        let Some(value) = usage.value.and_then(|v| Decimal::try_from(v).ok()) else {
            continue;
        };
        let (window_start, window_end) = match query.grain {
            None => (query.period.start, query.period.end),
            Some(_) => match (
                usage.window_start.as_ref().and_then(timestamp_to_datetime),
                usage.window_end.as_ref().and_then(timestamp_to_datetime),
            ) {
                (Some(start), Some(end)) => (start, end),
                _ => continue,
            },
        };
        let dimensions: HashMap<String, String> = usage
            .dimensions
            .into_iter()
            .filter_map(|(k, v)| v.value.filter(|s| !s.is_empty()).map(|v| (k, v)))
            .collect();
        let customer_id = match (&query.breakdown, usage.customer_id) {
            (MetricUsageBreakdown::Customer, Some(id)) if !usage.other => {
                Some(CustomerId::from_proto(id).map_err(invalid_metering_id)?)
            }
            (MetricUsageBreakdown::Customer, _) if !usage.other => {
                return Err(Report::new(StoreError::MeteringServiceError)
                    .attach("a customer's usage row carries no customer"));
            }
            _ => None,
        };
        points.push(MetricUsagePoint {
            window_start,
            window_end,
            value: query.metric.unrounded_units(value),
            dimensions,
            customer_id,
            other: usage.other,
        });
    }
    points.sort_by_key(|p| p.window_start);
    Ok(points)
}

fn timestamp_to_datetime(ts: &prost_types::Timestamp) -> Option<NaiveDateTime> {
    chrono::DateTime::from_timestamp(ts.seconds, u32::try_from(ts.nanos).ok()?)
        .map(|dt| dt.naive_utc())
}

#[derive(Clone)]
pub struct MeteringUsageClient {
    usage_grpc_client: UsageQueryServiceClient<LayeredClientService>,
    ingest_grpc_service: InternalEventsServiceClient<LayeredClientService>,
    /// Set once the store exists (the store holds this client); shared by every clone.
    tenant_lookup: Arc<OnceLock<KeepLastEventsLookup>>,
}

impl MeteringUsageClient {
    pub fn new(
        usage_grpc_client: UsageQueryServiceClient<LayeredClientService>,
        ingest_grpc_service: InternalEventsServiceClient<LayeredClientService>,
    ) -> Self {
        Self {
            usage_grpc_client,
            ingest_grpc_service,
            tenant_lookup: Arc::default(),
        }
    }

    /// Lets usage queries read the tenant's `keep_last_events` (ADR-0011 §4).
    pub fn bind_tenant_lookup(&self, lookup: KeepLastEventsLookup) {
        let _ = self.tenant_lookup.set(lookup);
    }

    /// Unbound (a client without a store, in tests) reads as a default tenant.
    async fn keep_last_events(&self, tenant_id: &TenantId) -> StoreResult<bool> {
        match self.tenant_lookup.get() {
            Some(lookup) => lookup.get(*tenant_id).await,
            None => {
                static UNBOUND: Once = Once::new();
                UNBOUND.call_once(|| {
                    log::error!(
                        "MeteringUsageClient has no tenant lookup bound, reading every tenant without keep-last"
                    )
                });
                Ok(false)
            }
        }
    }
}

impl MeteringUsageClient {
    async fn query_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: Option<&str>,
        only: Option<(&str, &str)>,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_string()));
        }

        let mut request = meter_request(
            tenant_id,
            self.keep_last_events(tenant_id).await?,
            customer_id,
            metric,
            group_by,
            QueryWindowSize::AggregateAll,
            &period,
        );
        if let Some((property, value)) = only {
            request.filters.push(PropertyFilter {
                property: property.to_owned(),
                op: property_filter::FilterOp::In as i32,
                values: vec![value.to_owned()],
            });
        }

        let mut metering_client_mut = self.usage_grpc_client.clone();
        let attributed = request.attribution.is_some();
        let response: QueryMeterResponse = match tokio::time::timeout(
            GRPC_TIMEOUT,
            metering_client_mut.query_meter(request),
        )
        .await
        {
            Ok(result) => result
                .change_context(StoreError::MeteringServiceError)
                .attach("Failed to query meter")?
                .into_inner(),
            Err(_) => {
                log::error!(
                    "query_meter timed out after {} seconds",
                    GRPC_TIMEOUT.as_secs()
                );
                return Err(error_stack::Report::new(StoreError::MeteringServiceError)
                    .attach("query_meter timed out"));
            }
        };
        ensure_attributed(attributed, &response)?;

        let data: Vec<GroupedUsageData> = response
            .usage
            .into_iter()
            .filter_map(|usage| {
                let value: Decimal = usage.value.and_then(|v| v.try_into().ok())?;

                Some(GroupedUsageData {
                    value,
                    dimensions: usage
                        .dimensions
                        .into_iter()
                        .filter_map(|(k, v)| v.value.filter(|s| !s.is_empty()).map(|v| (k, v)))
                        .collect(),
                })
            })
            .collect();

        Ok(UsageData { data, period })
    }
}

#[async_trait::async_trait]
impl UsageClient for MeteringUsageClient {
    async fn fetch_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.query_usage(tenant_id, customer_id, metric, None, None, period)
            .await
    }

    async fn fetch_usage_grouped_by(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: &str,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.query_usage(tenant_id, customer_id, metric, Some(group_by), None, period)
            .await
    }

    async fn fetch_usage_of_group(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: &str,
        value: &str,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.query_usage(
            tenant_id,
            customer_id,
            metric,
            Some(group_by),
            Some((group_by, value)),
            period,
        )
        .await
    }

    async fn fetch_usage_series(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: Option<&str>,
        grain: TenantUsageGrain,
        period: UsagePeriod,
    ) -> StoreResult<Vec<UsageSeriesPoint>> {
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_string()));
        }

        let window_size = match grain {
            TenantUsageGrain::Hour => QueryWindowSize::Hour,
            TenantUsageGrain::Day => QueryWindowSize::Day,
        };
        let request = meter_request(
            tenant_id,
            self.keep_last_events(tenant_id).await?,
            customer_id,
            metric,
            group_by,
            window_size,
            &period,
        );

        let mut client = self.usage_grpc_client.clone();
        let attributed = request.attribution.is_some();
        let response: QueryMeterResponse =
            match tokio::time::timeout(GRPC_TIMEOUT, client.query_meter(request)).await {
                Ok(result) => result
                    .change_context(StoreError::MeteringServiceError)
                    .attach("Failed to query meter (series)")?
                    .into_inner(),
                Err(_) => {
                    log::error!(
                        "query_meter (series) timed out after {} seconds",
                        GRPC_TIMEOUT.as_secs()
                    );
                    return Err(error_stack::Report::new(StoreError::MeteringServiceError)
                        .attach("query_meter (series) timed out"));
                }
            };
        ensure_attributed(attributed, &response)?;

        Ok(response
            .usage
            .into_iter()
            .filter_map(|usage| {
                let value: Decimal = usage.value.and_then(|v| v.try_into().ok())?;
                let window_start = usage
                    .window_start
                    .as_ref()
                    .and_then(timestamp_to_datetime)?;
                Some(UsageSeriesPoint {
                    window_start,
                    value,
                    dimensions: usage
                        .dimensions
                        .into_iter()
                        .filter_map(|(k, v)| v.value.filter(|s| !s.is_empty()).map(|v| (k, v)))
                        .collect(),
                })
            })
            .collect())
    }

    async fn list_metric_events(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
        after: Option<MetricEventCursor>,
        limit: u32,
    ) -> StoreResult<Vec<MetricEvent>> {
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_string()));
        }
        ensure_customer_wide(metric, "listing a metric's events")?;

        let request = QueryRawEventsRequest {
            tenant_id: tenant_id.to_string(),
            keep_last_events: self.keep_last_events(tenant_id).await?,
            from: Some(datetime_to_timestamp(period.start)),
            to: Some(datetime_to_timestamp(period.end)),
            limit: limit.min(1000),
            offset: 0,
            search: None,
            event_codes: vec![metric.code.clone()],
            customer_ids: vec![customer_id.to_string()],
            filters: build_property_filters(metric.filters.as_ref()),
            sort_order: SortOrder::TimestampAsc as i32,
            after_timestamp_nanos: after.as_ref().map(|cursor| cursor.timestamp_nanos),
            after_id: after.map(|cursor| cursor.id),
        };

        let response = match tokio::time::timeout(
            GRPC_TIMEOUT,
            self.usage_grpc_client.clone().query_raw_events(request),
        )
        .await
        {
            Ok(result) => result
                .change_context(StoreError::MeteringServiceError)
                .attach("Failed to list metric events")?
                .into_inner(),
            Err(_) => {
                log::error!(
                    "query_raw_events (metric) timed out after {} seconds",
                    GRPC_TIMEOUT.as_secs()
                );
                return Err(error_stack::Report::new(StoreError::MeteringServiceError)
                    .attach("query_raw_events (metric) timed out"));
            }
        };

        response
            .events
            .into_iter()
            .map(|event| {
                let timestamp = chrono::DateTime::parse_from_rfc3339(&event.timestamp)
                    .map_err(|e| {
                        error_stack::Report::new(StoreError::MeteringServiceError)
                            .attach(format!("invalid event timestamp: {e}"))
                    })?
                    .naive_utc();
                Ok(MetricEvent {
                    id: event.id,
                    timestamp,
                    properties: event.properties,
                })
            })
            .collect()
    }

    async fn fetch_total_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<Decimal> {
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_string()));
        }

        let request = QueryMeterRequest {
            tenant_id: tenant_id.as_proto(),
            keep_last_events: self.keep_last_events(tenant_id).await?,
            meter_slug: metric.id.to_string(),
            code: metric.code.clone(),
            meter_aggregation_type: map_aggregation_type(&metric.aggregation_type),
            customer_ids: vec![customer_id.to_string()],
            from: Some(datetime_to_timestamp(period.start)),
            to: Some(datetime_to_timestamp(period.end)),
            group_by_properties: vec![],
            window_size: QueryWindowSize::AggregateAll.into(),
            timezone: None,
            segmentation_filter: build_segmentation_filter(metric.segmentation_matrix.clone()),
            filters: build_property_filters(metric.filters.as_ref()),
            value_property: metric.aggregation_key.clone(),
            origin_filter: query_meter_request::OriginFilter::All.into(),
            merge_segments: true,
            attribution: build_attribution(&metric.usage_scope),
            ..Default::default()
        };

        let mut client = self.usage_grpc_client.clone();
        let attributed = request.attribution.is_some();
        let response: QueryMeterResponse =
            match tokio::time::timeout(GRPC_TIMEOUT, client.query_meter(request)).await {
                Ok(result) => result
                    .change_context(StoreError::MeteringServiceError)
                    .attach("Failed to query meter (total)")?
                    .into_inner(),
                Err(_) => {
                    log::error!(
                        "query_meter (total) timed out after {} seconds",
                        GRPC_TIMEOUT.as_secs()
                    );
                    return Err(error_stack::Report::new(StoreError::MeteringServiceError)
                        .attach("query_meter (total) timed out"));
                }
            };
        ensure_attributed(attributed, &response)?;

        let rows = response
            .usage
            .into_iter()
            .map(|usage| {
                usage
                    .value
                    .and_then(|v| Decimal::try_from(v).ok())
                    .ok_or_else(|| {
                        error_stack::Report::new(StoreError::MeteringServiceError)
                            .attach("query_meter (total) returned an unreadable value")
                    })
            })
            .collect::<StoreResult<Vec<Decimal>>>()?;
        // A metering that predates `merge_segments` answers one row per segment, which add up to
        // the total only for an additive aggregation.
        let additive = matches!(
            metric.aggregation_type,
            domain::enums::BillingMetricAggregateEnum::Sum
                | domain::enums::BillingMetricAggregateEnum::Count
        );
        match rows.as_slice() {
            [] => Ok(Decimal::ZERO),
            [total] => Ok(*total),
            _ if additive => rows.into_iter().try_fold(Decimal::ZERO, |sum, row| {
                sum.checked_add(row).ok_or_else(|| {
                    error_stack::Report::new(StoreError::InvalidDecimal)
                        .attach("usage total out of range")
                })
            }),
            _ => Err(error_stack::Report::new(StoreError::MeteringServiceError)
                .attach("query_meter (total) returned one row per segment")),
        }
    }

    async fn fetch_windowed_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<WindowedUsageData> {
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_string()));
        }

        let request = QueryMeterRequest {
            tenant_id: tenant_id.as_proto(),
            keep_last_events: self.keep_last_events(tenant_id).await?,
            meter_slug: metric.id.to_string(),
            code: metric.code.clone(),
            meter_aggregation_type: map_aggregation_type(&metric.aggregation_type),
            customer_ids: vec![customer_id.to_string()],
            from: Some(datetime_to_timestamp(period.start)),
            to: Some(datetime_to_timestamp(period.end)),
            group_by_properties: metric
                .usage_group_key
                .as_ref()
                .map(|k| vec![k.clone()])
                .unwrap_or_default(),
            window_size: QueryWindowSize::Day.into(),
            timezone: None,
            segmentation_filter: build_segmentation_filter(metric.segmentation_matrix.clone()),
            filters: build_property_filters(metric.filters.as_ref()),
            value_property: metric.aggregation_key.clone(),
            origin_filter: query_meter_request::OriginFilter::All.into(),
            merge_segments: false,
            attribution: build_attribution(&metric.usage_scope),
            ..Default::default()
        };

        let mut metering_client_mut = self.usage_grpc_client.clone();
        let attributed = request.attribution.is_some();
        let response: QueryMeterResponse = metering_client_mut
            .query_meter(request)
            .await
            .change_context(StoreError::MeteringServiceError)
            .attach("Failed to query meter (windowed)")?
            .into_inner();
        ensure_attributed(attributed, &response)?;

        let data: Vec<WindowedUsagePoint> = response
            .usage
            .into_iter()
            .filter_map(|usage| {
                let value: Decimal = usage.value.and_then(|v| v.try_into().ok())?;
                let window_start = usage.window_start.and_then(|ts| {
                    chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                        .map(|dt| dt.date_naive())
                })?;
                let window_end = usage.window_end.and_then(|ts| {
                    chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                        .map(|dt| dt.date_naive())
                })?;

                Some(WindowedUsagePoint {
                    window_start,
                    window_end,
                    value,
                    dimensions: usage
                        .dimensions
                        .into_iter()
                        .filter_map(|(k, v)| v.value.filter(|s| !s.is_empty()).map(|v| (k, v)))
                        .collect(),
                })
            })
            .collect();

        Ok(WindowedUsageData { data, period })
    }

    async fn count_events(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<u64> {
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_string()));
        }

        // The metric's own filters and segmentation select its events; the count ignores what it
        // aggregates.
        let request = QueryMeterRequest {
            tenant_id: tenant_id.as_proto(),
            keep_last_events: self.keep_last_events(tenant_id).await?,
            meter_slug: metric.id.to_string(),
            code: metric.code.clone(),
            meter_aggregation_type: AggregationType::Count as i32,
            customer_ids: vec![customer_id.to_string()],
            from: Some(datetime_to_timestamp(period.start)),
            to: Some(datetime_to_timestamp(period.end)),
            group_by_properties: vec![],
            window_size: QueryWindowSize::AggregateAll.into(),
            timezone: None,
            segmentation_filter: build_segmentation_filter(metric.segmentation_matrix.clone()),
            filters: build_property_filters(metric.filters.as_ref()),
            value_property: None,
            origin_filter: query_meter_request::OriginFilter::All.into(),
            merge_segments: true,
            attribution: build_attribution(&metric.usage_scope),
            ..Default::default()
        };

        let mut client = self.usage_grpc_client.clone();
        let attributed = request.attribution.is_some();
        let response: QueryMeterResponse =
            match tokio::time::timeout(GRPC_TIMEOUT, client.query_meter(request)).await {
                Ok(result) => result
                    .change_context(StoreError::MeteringServiceError)
                    .attach("Failed to query meter (count)")?
                    .into_inner(),
                Err(_) => {
                    return Err(error_stack::Report::new(StoreError::MeteringServiceError)
                        .attach("query_meter (count) timed out"));
                }
            };
        ensure_attributed(attributed, &response)?;

        // One row, or one per segment from a metering that predates `merge_segments`: the events are
        // their sum. A row that does not read as a count makes the whole count unknown rather than low.
        let mut total: u64 = 0;
        for usage in response.usage {
            let count = usage
                .value
                .and_then(|v| Decimal::try_from(v).ok())
                .and_then(|v| v.to_u64())
                .ok_or_else(|| {
                    error_stack::Report::new(StoreError::MeteringServiceError)
                        .attach("query_meter (count) returned an unreadable count")
                })?;
            total = total.saturating_add(count);
        }
        Ok(total)
    }

    async fn discover_properties(
        &self,
        tenant_id: &TenantId,
        event_code: &str,
        lookback_days: u32,
        limit: u32,
    ) -> StoreResult<Vec<DiscoveredProperty>> {
        let request = DiscoverPropertiesRequest {
            tenant_id: tenant_id.as_proto(),
            event_code: event_code.to_string(),
            lookback_days,
            limit,
        };

        let mut client = self.usage_grpc_client.clone();
        let response = discovery_response(
            "discover_properties",
            tokio::time::timeout(GRPC_TIMEOUT, client.discover_properties(request)).await,
        )?;

        Ok(response
            .properties
            .into_iter()
            .map(|p| DiscoveredProperty {
                key: p.key,
                cardinality: p.cardinality,
                sample_values: p.sample_values,
                numeric_ratio: p.numeric_ratio,
            })
            .collect())
    }

    async fn discover_event_codes(
        &self,
        tenant_id: &TenantId,
        search: &str,
        limit: u32,
    ) -> StoreResult<Vec<EventCodeCount>> {
        let request = DiscoverEventCodesRequest {
            tenant_id: tenant_id.as_proto(),
            search: search.to_string(),
            limit,
        };

        let mut client = self.usage_grpc_client.clone();
        let response = discovery_response(
            "discover_event_codes",
            tokio::time::timeout(GRPC_TIMEOUT, client.discover_event_codes(request)).await,
        )?;

        Ok(response
            .codes
            .into_iter()
            .map(|c| EventCodeCount {
                code: c.code,
                count: c.count,
            })
            .collect())
    }

    async fn discover_property_values(
        &self,
        tenant_id: &TenantId,
        event_code: &str,
        property_key: &str,
        search: &str,
        limit: u32,
    ) -> StoreResult<Vec<PropertyValueCount>> {
        let request = DiscoverPropertyValuesRequest {
            tenant_id: tenant_id.as_proto(),
            event_code: event_code.to_string(),
            property_key: property_key.to_string(),
            search: search.to_string(),
            limit,
        };

        let mut client = self.usage_grpc_client.clone();
        let response = discovery_response(
            "discover_property_values",
            tokio::time::timeout(GRPC_TIMEOUT, client.discover_property_values(request)).await,
        )?;

        Ok(response
            .values
            .into_iter()
            .map(|v| PropertyValueCount {
                value: v.value,
                count: v.count,
            })
            .collect())
    }

    async fn profile_event_properties(
        &self,
        tenant_id: &TenantId,
        event_code: &str,
        from_ts: i64,
        to_ts: i64,
        top_values: u32,
        limit: u32,
    ) -> StoreResult<EventPropertiesProfile> {
        let request = ProfileEventPropertiesRequest {
            tenant_id: tenant_id.as_proto(),
            event_code: event_code.to_string(),
            from_ts,
            to_ts,
            top_values,
            limit,
        };

        let mut client = self.usage_grpc_client.clone();
        let response = discovery_response(
            "profile_event_properties",
            tokio::time::timeout(GRPC_TIMEOUT, client.profile_event_properties(request)).await,
        )?;

        let properties = response
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

        Ok(EventPropertiesProfile {
            total_events: response.total_events,
            properties,
        })
    }

    async fn fetch_tenant_usage(
        &self,
        tenant_id: &TenantId,
        metric: &BillableMetric,
        period: UsagePeriod,
        grain: TenantUsageGrain,
    ) -> StoreResult<Vec<TenantUsagePoint>> {
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_string()));
        }
        ensure_customer_wide(metric, "tenant-wide usage")?;

        let window_size = match grain {
            TenantUsageGrain::Hour => QueryWindowSize::Hour,
            TenantUsageGrain::Day => QueryWindowSize::Day,
        };

        let request = QueryMeterRequest {
            tenant_id: tenant_id.as_proto(),
            keep_last_events: self.keep_last_events(tenant_id).await?,
            meter_slug: metric.id.to_string(),
            code: metric.code.clone(),
            meter_aggregation_type: map_aggregation_type(&metric.aggregation_type),
            // Empty = aggregate across all customers in the tenant.
            customer_ids: vec![],
            from: Some(datetime_to_timestamp(period.start)),
            to: Some(datetime_to_timestamp(period.end)),
            // No per-dimension breakdown: a single tenant-wide series.
            group_by_properties: vec![],
            window_size: window_size.into(),
            timezone: None,
            segmentation_filter: build_segmentation_filter(metric.segmentation_matrix.clone()),
            filters: build_property_filters(metric.filters.as_ref()),
            value_property: metric.aggregation_key.clone(),
            origin_filter: query_meter_request::OriginFilter::All.into(),
            merge_segments: true,
            attribution: None,
            ..Default::default()
        };

        let mut client = self.usage_grpc_client.clone();
        let response: QueryMeterResponse =
            match tokio::time::timeout(GRPC_TIMEOUT, client.query_meter(request)).await {
                Ok(result) => result
                    .change_context(StoreError::MeteringServiceError)
                    .attach("Failed to query meter (tenant usage)")?
                    .into_inner(),
                Err(_) => {
                    log::error!(
                        "query_meter (tenant usage) timed out after {} seconds",
                        GRPC_TIMEOUT.as_secs()
                    );
                    return Err(error_stack::Report::new(StoreError::MeteringServiceError)
                        .attach("query_meter (tenant usage) timed out"));
                }
            };

        let mut points: Vec<TenantUsagePoint> = response
            .usage
            .into_iter()
            .filter_map(|usage| {
                let value: Decimal = usage.value.and_then(|v| v.try_into().ok())?;
                let window_start = usage.window_start.and_then(|ts| {
                    chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                        .map(|dt| dt.naive_utc())
                })?;
                let window_end = usage.window_end.and_then(|ts| {
                    chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                        .map(|dt| dt.naive_utc())
                })?;

                Some(TenantUsagePoint {
                    window_start,
                    window_end,
                    value,
                })
            })
            .collect();

        points.sort_by_key(|p| p.window_start);

        Ok(points)
    }

    async fn query_metric_usage(
        &self,
        tenant_id: &TenantId,
        query: MetricUsageQuery<'_>,
    ) -> StoreResult<Vec<MetricUsagePoint>> {
        let request =
            metric_usage_request(tenant_id, self.keep_last_events(tenant_id).await?, &query)?;

        let mut client = self.usage_grpc_client.clone();
        let response = match tokio::time::timeout(GRPC_TIMEOUT, client.query_meter(request)).await {
            Ok(result) => result
                .change_context(StoreError::MeteringServiceError)
                .attach("Failed to query meter (metric usage)")?
                .into_inner(),
            Err(_) => {
                return Err(Report::new(StoreError::MeteringServiceError)
                    .attach("query_meter (metric usage) timed out"));
            }
        };

        metric_usage_points(&query, response)
    }

    async fn fetch_usage_summary(
        &self,
        tenant_id: &TenantId,
        customer_id: Option<&CustomerId>,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_string()));
        }

        if customer_id.is_none() {
            ensure_customer_wide(metric, "usage across customers")?;
        }
        let customer_ids = customer_id
            .map(|id| vec![id.to_string()])
            .unwrap_or_default();

        let request = QueryMeterRequest {
            tenant_id: tenant_id.as_proto(),
            keep_last_events: self.keep_last_events(tenant_id).await?,
            meter_slug: metric.id.to_string(),
            code: metric.code.clone(),
            meter_aggregation_type: map_aggregation_type(&metric.aggregation_type),
            customer_ids,
            from: Some(datetime_to_timestamp(period.start)),
            to: Some(datetime_to_timestamp(period.end)),
            group_by_properties: metric
                .usage_group_key
                .as_ref()
                .map(|k| vec![k.clone()])
                .unwrap_or_default(),
            window_size: QueryWindowSize::AggregateAll.into(),
            timezone: None,
            segmentation_filter: build_segmentation_filter(metric.segmentation_matrix.clone()),
            filters: build_property_filters(metric.filters.as_ref()),
            value_property: metric.aggregation_key.clone(),
            origin_filter: query_meter_request::OriginFilter::All.into(),
            merge_segments: false,
            attribution: build_attribution(&metric.usage_scope),
            ..Default::default()
        };

        let mut metering_client_mut = self.usage_grpc_client.clone();
        let attributed = request.attribution.is_some();
        let response: QueryMeterResponse = metering_client_mut
            .query_meter(request)
            .await
            .change_context(StoreError::MeteringServiceError)
            .attach("Failed to query meter (summary)")?
            .into_inner();
        ensure_attributed(attributed, &response)?;

        let data: Vec<GroupedUsageData> = response
            .usage
            .into_iter()
            .filter_map(|usage| {
                let value: Decimal = usage.value.and_then(|v| v.try_into().ok())?;

                Some(GroupedUsageData {
                    value,
                    dimensions: usage
                        .dimensions
                        .into_iter()
                        .filter_map(|(k, v)| v.value.filter(|s| !s.is_empty()).map(|v| (k, v)))
                        .collect(),
                })
            })
            .collect();

        Ok(UsageData { data, period })
    }

    async fn search_events(
        &self,
        tenant_id: &TenantId,
        options: EventSearchOptions,
    ) -> StoreResult<EventSearchResult> {
        let metering_request = QueryRawEventsRequest {
            tenant_id: tenant_id.to_string(),
            keep_last_events: self.keep_last_events(tenant_id).await?,
            from: options.from,
            to: options.to,
            limit: options.limit.min(1000), // Cap at 1000
            offset: options.offset,
            search: options.search,
            event_codes: options.event_codes,
            customer_ids: options.customer_ids,
            filters: options.filters,
            sort_order: options.sort_order,
            after_timestamp_nanos: None,
            after_id: None,
        };

        let response = match tokio::time::timeout(
            GRPC_TIMEOUT,
            self.usage_grpc_client
                .clone()
                .query_raw_events(metering_request),
        )
        .await
        {
            Ok(result) => result
                .change_context(StoreError::MeteringServiceError)
                .attach("Failed to search events")?,
            Err(_) => {
                log::error!(
                    "query_raw_events timed out after {} seconds",
                    GRPC_TIMEOUT.as_secs()
                );
                return Err(error_stack::Report::new(StoreError::MeteringServiceError)
                    .attach("query_raw_events timed out"));
            }
        };

        let metering_response = response.into_inner();

        Ok(EventSearchResult {
            events: metering_response.events,
        })
    }

    async fn ingest_events(
        &self,
        tenant_id: &TenantId,
        request: meteroid_store::clients::usage::IngestEventsRequest,
    ) -> StoreResult<meteroid_store::clients::usage::IngestEventsResult> {
        let grpc_request = InternalIngestRequest {
            tenant_id: tenant_id.to_string(),
            events: request.events,
            allow_backfilling: request.allow_backfilling,
            fail_on_error: request.fail_on_error,
            keep_last_events: request.keep_last_events,
        };

        let response = match tokio::time::timeout(
            GRPC_TIMEOUT,
            self.ingest_grpc_service
                .clone()
                .ingest_internal(grpc_request),
        )
        .await
        {
            Ok(result) => result
                .change_context(StoreError::MeteringServiceError)
                .attach("Failed to ingest events")?,
            Err(_) => {
                log::error!(
                    "ingest_events timed out after {} seconds",
                    GRPC_TIMEOUT.as_secs()
                );
                return Err(error_stack::Report::new(StoreError::MeteringServiceError)
                    .attach("ingest_events timed out"));
            }
        };

        let metering_response = response.into_inner();

        Ok(meteroid_store::clients::usage::IngestEventsResult {
            failures: metering_response
                .failures
                .into_iter()
                .map(|f| meteroid_store::clients::usage::IngestEventsFailure {
                    event_id: f.event_id,
                    reason: f.reason,
                })
                .collect(),
        })
    }

    async fn purge_tenants(&self, tenant_ids: &[TenantId]) -> StoreResult<()> {
        if tenant_ids.is_empty() {
            return Ok(());
        }
        let ids: Vec<String> = tenant_ids.iter().map(|t| t.as_proto()).collect();

        let mut metering_client = self.usage_grpc_client.clone();
        metering_client
            .purge_tenants(MeteringPurgeTenantsRequest { tenant_ids: ids })
            .await
            .change_context(StoreError::MeteringServiceError)
            .attach("Failed to purge tenants metering events")?;

        Ok(())
    }

    async fn poll_usage_activity(
        &self,
        since: chrono::DateTime<chrono::Utc>,
        after: Option<ActivityPosition>,
        limit: u32,
    ) -> StoreResult<Vec<CustomerActivity>> {
        let mut client = self.usage_grpc_client.clone();

        let response = match tokio::time::timeout(
            GRPC_TIMEOUT,
            client.poll_usage_activity(PollUsageActivityRequest {
                since: Some(prost_types::Timestamp {
                    seconds: since.timestamp(),
                    nanos: since.timestamp_subsec_nanos() as i32,
                }),
                after: after.map(|after| poll_usage_activity_request::ActivityPosition {
                    last_seen_at: Some(prost_types::Timestamp {
                        seconds: after.last_seen_at.timestamp(),
                        nanos: after.last_seen_at.timestamp_subsec_nanos() as i32,
                    }),
                    tenant_id: after.tenant_id.as_proto(),
                    customer_id: after.customer_id.as_proto(),
                }),
                // One shard of one: every tenant's activity. Metering still filters by shard.
                shard_index: 0,
                shard_count: 1,
                limit,
            }),
        )
        .await
        {
            Ok(result) => result
                .change_context(StoreError::MeteringServiceError)
                .attach("Failed to poll usage activity")?
                .into_inner(),
            Err(_) => {
                return Err(error_stack::Report::new(StoreError::MeteringServiceError)
                    .attach("poll_usage_activity timed out"));
            }
        };

        response
            .customers
            .into_iter()
            .map(|c| {
                let timestamp = |ts: Option<prost_types::Timestamp>, field: &str| {
                    ts.and_then(|ts| chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32))
                        .ok_or_else(|| {
                            error_stack::Report::new(StoreError::MeteringServiceError)
                                .attach(format!("usage activity row without {field}"))
                        })
                };
                let last_seen_at = timestamp(c.last_seen_at, "last_seen_at")?;
                let min_event_at = timestamp(c.min_event_at, "min_event_at")?;
                Ok(CustomerActivity {
                    tenant_id: TenantId::from_proto(c.tenant_id).map_err(invalid_metering_id)?,
                    customer_id: CustomerId::from_proto(c.customer_id)
                        .map_err(invalid_metering_id)?,
                    last_seen_at,
                    min_event_at,
                })
            })
            .collect()
    }

    async fn list_tenant_first_events(
        &self,
        tenants: &[TenantEventScan],
    ) -> StoreResult<Vec<TenantFirstIngestedEvent>> {
        use metering_grpc::meteroid::metering::v1::list_tenant_first_events_request::TenantScan;

        let mut client = self.usage_grpc_client.clone();

        let response = client
            .list_tenant_first_events(ListTenantFirstEventsRequest {
                tenants: tenants
                    .iter()
                    .map(|t| TenantScan {
                        tenant_id: t.tenant_id.as_proto(),
                        from: t.from.as_ref().map(ProtoConv::as_proto),
                    })
                    .collect(),
            })
            .await
            .change_context(StoreError::MeteringServiceError)
            .attach("Failed to list tenant first events")?
            .into_inner();

        Ok(response
            .tenants
            .into_iter()
            .filter_map(|t| {
                let tenant_id = TenantId::from_proto(t.tenant_id).ok()?;
                let first_ingested_at = t.first_ingested_at.and_then(|ts| {
                    chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                        .map(|dt| dt.naive_utc())
                })?;
                Some(TenantFirstIngestedEvent {
                    tenant_id,
                    first_ingested_at,
                })
            })
            .collect())
    }

    async fn list_customer_first_events(
        &self,
        tenant_id: &TenantId,
        customer_ids: &[CustomerId],
    ) -> StoreResult<Vec<CustomerFirstIngestedEvent>> {
        let mut client = self.usage_grpc_client.clone();

        let response = client
            .list_customer_first_events(ListCustomerFirstEventsRequest {
                tenant_id: tenant_id.as_proto(),
                customer_ids: customer_ids.iter().map(|id| id.as_proto()).collect(),
            })
            .await
            .change_context(StoreError::MeteringServiceError)
            .attach("Failed to list customer first events")?
            .into_inner();

        Ok(response
            .customers
            .into_iter()
            .filter_map(|c| {
                let customer_id = CustomerId::from_proto(c.customer_id).ok()?;
                let first_ingested_at = chrono::DateTime::parse_from_rfc3339(&c.first_ingested_at)
                    .ok()?
                    .naive_utc();
                Some(CustomerFirstIngestedEvent {
                    customer_id,
                    first_ingested_at,
                })
            })
            .collect())
    }
}

#[async_trait::async_trait]
impl PendingUsageSource for MeteringUsageClient {
    async fn usage_since(
        &self,
        request: PendingUsageRequest<'_>,
    ) -> StoreResult<Vec<CustomerUsage>> {
        if request.since >= request.until {
            bail!(StoreError::InvalidArgument("invalid window".to_string()));
        }
        if request.customer_ids.is_empty() {
            return Ok(vec![]);
        }

        let metric = request.metric;
        let origin_filter = match request.origin {
            OriginFilter::All => query_meter_request::OriginFilter::All,
        };

        let grpc_request = QueryMeterRequest {
            tenant_id: request.tenant_id.as_proto(),
            keep_last_events: request.keep_last_events,
            meter_slug: metric.id.to_string(),
            code: metric.code.clone(),
            meter_aggregation_type: map_aggregation_type(&metric.aggregation_type),
            customer_ids: request
                .customer_ids
                .iter()
                .map(|id| id.as_proto())
                .collect(),
            from: Some(datetime_to_timestamp(request.since)),
            to: Some(datetime_to_timestamp(request.until)),
            group_by_properties: metric
                .usage_group_key
                .iter()
                .map(String::as_str)
                .chain(request.group_by)
                .fold(Vec::<String>::new(), |mut names, name| {
                    if !names.iter().any(|known| known == name) {
                        names.push(name.to_owned());
                    }
                    names
                }),
            window_size: QueryWindowSize::AggregateAll.into(),
            timezone: None,
            segmentation_filter: build_segmentation_filter(metric.segmentation_matrix.clone()),
            filters: build_property_filters(metric.filters.as_ref()),
            value_property: metric.aggregation_key.clone(),
            origin_filter: origin_filter.into(),
            merge_segments: false,
            attribution: build_attribution(&metric.usage_scope),
            ..Default::default()
        };

        let mut client = self.usage_grpc_client.clone();
        let attributed = grpc_request.attribution.is_some();
        let response: QueryMeterResponse =
            match tokio::time::timeout(GRPC_TIMEOUT, client.query_meter(grpc_request)).await {
                Ok(result) => result
                    .change_context(StoreError::MeteringServiceError)
                    .attach("Failed to query meter (pending usage)")?
                    .into_inner(),
                Err(_) => {
                    return Err(error_stack::Report::new(StoreError::MeteringServiceError)
                        .attach("query_meter (pending usage) timed out"));
                }
            };
        ensure_attributed(attributed, &response)?;

        let mut by_customer: HashMap<CustomerId, Vec<GroupedUsageData>> = HashMap::new();
        for usage in response.usage {
            let Some(customer_id) = usage.customer_id else {
                continue;
            };
            let customer_id = CustomerId::from_proto(customer_id).map_err(invalid_metering_id)?;
            let Some(value) = usage.value.and_then(|v| Decimal::try_from(v).ok()) else {
                continue;
            };
            let group = GroupedUsageData {
                value,
                dimensions: usage
                    .dimensions
                    .into_iter()
                    .filter_map(|(k, v)| v.value.filter(|s| !s.is_empty()).map(|v| (k, v)))
                    .collect(),
            };
            by_customer.entry(customer_id).or_default().push(group);
        }

        Ok(by_customer
            .into_iter()
            .map(|(customer_id, groups)| CustomerUsage {
                customer_id,
                groups,
            })
            .collect())
    }
}

fn invalid_metering_id(e: impl std::fmt::Display) -> error_stack::Report<StoreError> {
    error_stack::Report::new(StoreError::MeteringServiceError)
        .attach(format!("invalid id in metering response: {e}"))
}

pub fn datetime_to_timestamp(dt: NaiveDateTime) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: dt.and_utc().timestamp(),
        nanos: dt.nanosecond() as i32,
    }
}

#[cfg(test)]
mod build_property_filters_tests {
    use super::*;
    use domain::billable_metrics::{MetricFilter, MetricFilterOp, MetricFilters};

    fn collapse(op: MetricFilterOp) -> PropertyFilter {
        let out = build_property_filters(Some(&MetricFilters(vec![MetricFilter {
            property: "model".to_string(),
            op,
            values: vec!["gpt_4".to_string()],
        }])));
        assert_eq!(out.len(), 1);
        out.into_iter().next().unwrap()
    }

    #[test]
    fn none_and_empty_yield_no_filters() {
        assert!(build_property_filters(None).is_empty());
        assert!(build_property_filters(Some(&MetricFilters(vec![]))).is_empty());
    }

    #[test]
    fn equal_and_in_collapse_to_engine_in() {
        for op in [MetricFilterOp::Equal, MetricFilterOp::In] {
            let f = collapse(op);
            assert_eq!(f.op, property_filter::FilterOp::In as i32);
            assert_eq!(f.property, "model");
            assert_eq!(f.values, vec!["gpt_4".to_string()]);
        }
    }

    #[test]
    fn not_equal_and_not_in_collapse_to_engine_not_in() {
        for op in [MetricFilterOp::NotEqual, MetricFilterOp::NotIn] {
            assert_eq!(collapse(op).op, property_filter::FilterOp::NotIn as i32);
        }
    }
}

#[cfg(test)]
mod attribution_tests {
    use super::*;
    use chrono::NaiveDate;
    use meteroid_store::domain::{ScopeSegment, UsageClaim, UsageFilter};

    fn at(day: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    #[test]
    fn a_customer_wide_read_sends_no_attribution() {
        assert_eq!(build_attribution(&UsageScope::default()), None);
    }

    #[test]
    fn a_scoped_read_sends_its_segments() {
        let filter = UsageFilter::new("api_key".to_owned(), "key-A".to_owned()).unwrap();
        let attribution = build_attribution(&UsageScope {
            segments: vec![ScopeSegment {
                from: Some(at(1)),
                until: None,
                matching: None,
                excluded: vec![UsageClaim {
                    filter,
                    from: at(1),
                    until: Some(at(15)),
                }],
            }],
        })
        .unwrap();
        assert_eq!(attribution.segments.len(), 1);
        let segment = &attribution.segments[0];
        assert_eq!(segment.from, Some(datetime_to_timestamp(at(1))));
        assert_eq!(segment.to, None);
        assert_eq!(segment.matching, None);
        let claim = &segment.excluded[0];
        assert_eq!(
            (claim.property.as_str(), claim.value.as_str()),
            ("api_key", "key-A")
        );
        assert_eq!(claim.to, Some(datetime_to_timestamp(at(15))));
    }

    #[test]
    fn an_answer_that_ignored_the_attribution_is_refused() {
        let ignored = QueryMeterResponse {
            usage: vec![],
            attribution_applied: false,
            ..Default::default()
        };
        let applied = QueryMeterResponse {
            usage: vec![],
            attribution_applied: true,
            ..Default::default()
        };
        assert!(matches!(
            ensure_attributed(true, &ignored)
                .unwrap_err()
                .current_context(),
            StoreError::MeteringNotReady
        ));
        assert!(ensure_attributed(true, &applied).is_ok());
        assert!(ensure_attributed(false, &ignored).is_ok());
    }
}

#[cfg(test)]
mod metric_usage_tests {
    use super::*;
    use chrono::NaiveDate;
    use common_grpc::meteroid::common::v1::Decimal as ProtoDecimal;
    use metering_grpc::meteroid::metering::v1::query_meter_response::Usage;
    use meteroid_store::domain::MetricDefinition;
    use meteroid_store::domain::billable_metrics::{Dimension, SegmentationMatrix};
    use meteroid_store::domain::enums::BillingMetricAggregateEnum;

    fn at(day: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    fn metric(matrix: Option<SegmentationMatrix>) -> MetricDefinition {
        MetricDefinition {
            code: "api_calls".to_owned(),
            aggregation_type: BillingMetricAggregateEnum::Sum,
            aggregation_key: Some("bytes".to_owned()),
            unit_conversion_factor: Some(1000),
            segmentation_matrix: matrix,
            filters: None,
        }
    }

    fn query<'a>(
        metric: &'a MetricDefinition,
        customers: &'a [CustomerId],
        breakdown: MetricUsageBreakdown,
        top_n: Option<u32>,
    ) -> MetricUsageQuery<'a> {
        MetricUsageQuery {
            metric,
            customer_ids: customers,
            breakdown,
            grain: None,
            top_n,
            period: UsagePeriod {
                start: at(1),
                end: at(8),
            },
        }
    }

    fn usage(customer: Option<CustomerId>, value: &str) -> Usage {
        Usage {
            other: customer.is_none(),
            customer_id: customer.map(|id| id.as_proto()),
            value: Some(ProtoDecimal {
                value: value.to_owned(),
            }),
            window_start: Some(datetime_to_timestamp(at(2))),
            window_end: Some(datetime_to_timestamp(at(3))),
            dimensions: HashMap::new(),
        }
    }

    #[test]
    fn a_customer_breakdown_asks_metering_to_group_and_cap() {
        let metric = metric(Some(SegmentationMatrix::Single(Dimension {
            key: "region".to_owned(),
            values: vec!["eu".to_owned()],
        })));
        let request = metric_usage_request(
            &TenantId::default(),
            true,
            &query(&metric, &[], MetricUsageBreakdown::Customer, Some(8)),
        )
        .unwrap();
        assert!(request.group_by_customer);
        assert_eq!(request.top_n, Some(8));
        assert!(request.merge_segments);
        assert!(request.segmentation_filter.is_some());
        assert!(request.keep_last_events);
        assert_eq!(request.window_size(), QueryWindowSize::AggregateAll);
    }

    #[test]
    fn an_incomplete_matrix_restricts_nothing() {
        let metric = metric(Some(SegmentationMatrix::Single(Dimension {
            key: "region".to_owned(),
            values: vec![],
        })));
        let request = metric_usage_request(
            &TenantId::default(),
            false,
            &query(&metric, &[], MetricUsageBreakdown::None, None),
        )
        .unwrap();
        assert!(request.segmentation_filter.is_none());
    }

    #[test]
    fn several_customers_need_a_customer_breakdown() {
        let metric = metric(None);
        let customers = [
            CustomerId::from(uuid::Uuid::from_u128(1)),
            CustomerId::from(uuid::Uuid::from_u128(2)),
        ];
        let refused = metric_usage_request(
            &TenantId::default(),
            false,
            &query(&metric, &customers, MetricUsageBreakdown::None, None),
        );
        assert!(refused.is_err());
        assert!(
            metric_usage_request(
                &TenantId::default(),
                false,
                &query(&metric, &customers, MetricUsageBreakdown::Customer, None),
            )
            .is_ok()
        );
    }

    #[test]
    fn points_carry_the_converted_value_and_the_other_series() {
        let metric = metric(None);
        let customer = CustomerId::from(uuid::Uuid::from_u128(1));
        let response = QueryMeterResponse {
            usage: vec![usage(Some(customer), "2500"), usage(None, "500")],
            attribution_applied: true,
            grouping_applied: true,
        };
        let points = metric_usage_points(
            &query(&metric, &[], MetricUsageBreakdown::Customer, Some(1)),
            response,
        )
        .unwrap();
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].customer_id, Some(customer));
        assert_eq!(points[0].value, Decimal::new(25, 1));
        assert!(!points[0].other);
        assert_eq!(points[1].customer_id, None);
        assert!(points[1].other);
        // A whole-period read spans the requested range, not the first and last event.
        assert_eq!(
            (points[0].window_start, points[0].window_end),
            (at(1), at(8))
        );
    }

    #[test]
    fn only_the_marked_row_is_other_whatever_its_label() {
        let metric = metric(None);
        let labelled = |label: &str, other: bool| {
            let mut row = usage(Some(CustomerId::from(uuid::Uuid::from_u128(1))), "1000");
            row.other = other;
            row.dimensions = [(
                "tier".to_owned(),
                metering_grpc::meteroid::metering::v1::query_meter_response::usage::DimensionValueField {
                    value: Some(label.to_owned()),
                },
            )]
            .into();
            row
        };
        let response = QueryMeterResponse {
            usage: vec![labelled("Other", false), labelled("Other", true)],
            attribution_applied: true,
            grouping_applied: true,
        };
        let points = metric_usage_points(
            &query(
                &metric,
                &[],
                MetricUsageBreakdown::Property("tier".to_owned()),
                Some(1),
            ),
            response,
        )
        .unwrap();
        assert_eq!(
            points.iter().map(|p| p.other).collect::<Vec<_>>(),
            vec![false, true]
        );
        assert!(points.iter().all(|p| p.customer_id.is_none()));
    }

    #[test]
    fn a_metering_service_that_ignored_the_grouping_is_refused() {
        let metric = metric(None);
        let response = QueryMeterResponse {
            usage: vec![usage(None, "3000")],
            attribution_applied: true,
            grouping_applied: false,
        };
        let refused = metric_usage_points(
            &query(&metric, &[], MetricUsageBreakdown::Customer, Some(5)),
            response,
        )
        .unwrap_err();
        assert!(matches!(
            refused.current_context(),
            StoreError::MeteringNotReady
        ));
    }
}
