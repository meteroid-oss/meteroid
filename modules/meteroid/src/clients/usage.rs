use chrono::{NaiveDate, Timelike};
use common_domain::ids::{CustomerId, TenantId};
use common_grpc::middleware::client::LayeredClientService;

use error_stack::{ResultExt, bail};
use metering_grpc::meteroid::metering::v1::internal_events_service_client::InternalEventsServiceClient;
use metering_grpc::meteroid::metering::v1::meter::AggregationType;
use metering_grpc::meteroid::metering::v1::query_meter_request::QueryWindowSize;
use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;
use metering_grpc::meteroid::metering::v1::{
    Filter, InternalIngestRequest, QueryMeterRequest, QueryMeterResponse, QueryRawEventsRequest,
    SegmentationFilter, segmentation_filter,
    segmentation_filter::{
        IndependentFilters, LinkedFilters, linked_filters::LinkedDimensionValues,
    },
};
use meteroid_store::clients::usage::{
    EventSearchOptions, EventSearchResult, GroupedUsageData, UsageClient, UsageData,
    WindowedUsageData, WindowedUsagePoint,
};
use meteroid_store::domain::{BillableMetric, Period};
use meteroid_store::errors::StoreError;
use meteroid_store::{StoreResult, domain};
use rust_decimal::Decimal;
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

#[derive(Clone, Debug)]
pub struct MeteringUsageClient {
    usage_grpc_client: UsageQueryServiceClient<LayeredClientService>,
    ingest_grpc_service: InternalEventsServiceClient<LayeredClientService>,
}

impl MeteringUsageClient {
    pub fn new(
        usage_grpc_client: UsageQueryServiceClient<LayeredClientService>,
        ingest_grpc_service: InternalEventsServiceClient<LayeredClientService>,
    ) -> Self {
        Self {
            usage_grpc_client,
            ingest_grpc_service,
        }
    }
}

#[async_trait::async_trait]
impl UsageClient for MeteringUsageClient {
    async fn fetch_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: Period,
    ) -> StoreResult<UsageData> {
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_string()));
        }

        let request = QueryMeterRequest {
            tenant_id: tenant_id.as_proto(),
            meter_slug: metric.id.to_string(),
            code: metric.code.clone(),
            meter_aggregation_type: map_aggregation_type(&metric.aggregation_type),
            customer_ids: vec![customer_id.to_string()],
            from: Some(date_to_timestamp(period.start)),
            to: Some(date_to_timestamp(period.end)),
            group_by_properties: metric
                .usage_group_key
                .as_ref()
                .map(|k| vec![k.clone()])
                .unwrap_or_default(),
            window_size: QueryWindowSize::AggregateAll.into(),
            timezone: None,
            segmentation_filter: build_segmentation_filter(metric.segmentation_matrix.clone()),
            value_property: metric.aggregation_key.clone(),
        };

        let mut metering_client_mut = self.usage_grpc_client.clone();
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

    async fn fetch_windowed_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: Period,
    ) -> StoreResult<WindowedUsageData> {
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_string()));
        }

        let request = QueryMeterRequest {
            tenant_id: tenant_id.as_proto(),
            meter_slug: metric.id.to_string(),
            code: metric.code.clone(),
            meter_aggregation_type: map_aggregation_type(&metric.aggregation_type),
            customer_ids: vec![customer_id.to_string()],
            from: Some(date_to_timestamp(period.start)),
            to: Some(date_to_timestamp(period.end)),
            group_by_properties: metric
                .usage_group_key
                .as_ref()
                .map(|k| vec![k.clone()])
                .unwrap_or_default(),
            window_size: QueryWindowSize::Day.into(),
            timezone: None,
            segmentation_filter: build_segmentation_filter(metric.segmentation_matrix.clone()),
            value_property: metric.aggregation_key.clone(),
        };

        let mut metering_client_mut = self.usage_grpc_client.clone();
        let response: QueryMeterResponse = metering_client_mut
            .query_meter(request)
            .await
            .change_context(StoreError::MeteringServiceError)
            .attach("Failed to query meter (windowed)")?
            .into_inner();

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

    async fn fetch_usage_summary(
        &self,
        tenant_id: &TenantId,
        customer_id: Option<&CustomerId>,
        metric: &BillableMetric,
        period: Period,
    ) -> StoreResult<UsageData> {
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_string()));
        }

        let customer_ids = customer_id
            .map(|id| vec![id.to_string()])
            .unwrap_or_default();

        let request = QueryMeterRequest {
            tenant_id: tenant_id.as_proto(),
            meter_slug: metric.id.to_string(),
            code: metric.code.clone(),
            meter_aggregation_type: map_aggregation_type(&metric.aggregation_type),
            customer_ids,
            from: Some(date_to_timestamp(period.start)),
            to: Some(date_to_timestamp(period.end)),
            group_by_properties: metric
                .usage_group_key
                .as_ref()
                .map(|k| vec![k.clone()])
                .unwrap_or_default(),
            window_size: QueryWindowSize::AggregateAll.into(),
            timezone: None,
            segmentation_filter: build_segmentation_filter(metric.segmentation_matrix.clone()),
            value_property: metric.aggregation_key.clone(),
        };

        let mut metering_client_mut = self.usage_grpc_client.clone();
        let response: QueryMeterResponse = metering_client_mut
            .query_meter(request)
            .await
            .change_context(StoreError::MeteringServiceError)
            .attach("Failed to query meter (summary)")?
            .into_inner();

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
            from: options.from,
            to: options.to,
            limit: options.limit.min(1000), // Cap at 1000
            offset: options.offset,
            search: options.search,
            event_codes: options.event_codes,
            customer_ids: options.customer_ids,
            sort_order: options.sort_order,
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
}

fn date_to_timestamp(dt: NaiveDate) -> prost_types::Timestamp {
    let dt_at_start_of_day = dt.and_hms_opt(0, 0, 0).unwrap();
    prost_types::Timestamp {
        seconds: dt_at_start_of_day.and_utc().timestamp(),
        nanos: dt_at_start_of_day.nanosecond() as i32,
    }
}
