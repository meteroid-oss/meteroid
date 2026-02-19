use crate::api::billablemetrics::mapping;
use bytes::Buf;
use chrono::{NaiveDate, Timelike};
use common_domain::ids::{AliasOr, CustomerId, TenantId};
use common_grpc::middleware::client::LayeredClientService;
use csv::ReaderBuilder;
use error_stack::{ResultExt, bail};
use metering_grpc::meteroid::metering::v1::event::CustomerId as EventCustomerId;
use metering_grpc::meteroid::metering::v1::internal_events_service_client::InternalEventsServiceClient;
use metering_grpc::meteroid::metering::v1::meter::AggregationType;
use metering_grpc::meteroid::metering::v1::meters_service_client::MetersServiceClient;
use metering_grpc::meteroid::metering::v1::query_meter_request::QueryWindowSize;
use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;
use metering_grpc::meteroid::metering::v1::{
    Event, Filter, InternalIngestRequest, QueryMeterRequest, QueryMeterResponse,
    QueryRawEventsRequest, RegisterMeterRequest, SegmentationFilter, segmentation_filter,
    segmentation_filter::{
        IndependentFilters, LinkedFilters, linked_filters::LinkedDimensionValues,
    },
};
use meteroid_store::clients::usage::{
    CsvIngestionFailure, CsvIngestionOptions, CsvIngestionResult, EventSearchOptions,
    EventSearchResult, GroupedUsageData, Metadata, UsageClient, UsageData,
};
use meteroid_store::domain::{BillableMetric, Period};
use meteroid_store::errors::StoreError;
use meteroid_store::{StoreResult, domain};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;
use tonic::Request;

const MAX_CSV_SIZE: usize = 10 * 1024 * 1024; // 10MB limit
const MAX_BATCH_SIZE: usize = 500; // Max events per batch

fn extract_error_message(status: &tonic::Status) -> String {
    status.message().to_string()
}

#[derive(Clone, Debug)]
pub struct MeteringUsageClient {
    usage_grpc_client: UsageQueryServiceClient<LayeredClientService>,
    meters_grpc_client: MetersServiceClient<LayeredClientService>,
    ingest_grpc_service: InternalEventsServiceClient<LayeredClientService>,
}

impl MeteringUsageClient {
    pub fn new(
        usage_grpc_client: UsageQueryServiceClient<LayeredClientService>,
        meters_grpc_client: MetersServiceClient<LayeredClientService>,
        ingest_grpc_service: InternalEventsServiceClient<LayeredClientService>,
    ) -> Self {
        Self {
            usage_grpc_client,
            meters_grpc_client,
            ingest_grpc_service,
        }
    }
}

#[async_trait::async_trait]
impl UsageClient for MeteringUsageClient {
    async fn register_meter(
        &self,
        tenant_id: TenantId,
        metric: &BillableMetric,
    ) -> StoreResult<Vec<Metadata>> {
        let metering_meter = mapping::metric::domain_to_metering(metric.clone());

        let response = self
            .meters_grpc_client
            .clone()
            .register_meter(Request::new(RegisterMeterRequest {
                meter: Some(metering_meter),
                tenant_id: tenant_id.to_string(),
            }))
            // TODO add in db/response the register , error and allow retrying
            .await
            .map(tonic::Response::into_inner)
            .change_context(StoreError::MeteringServiceError)
            .attach("Failed to register meter")?;

        let metadata = response
            .metadata
            .into_iter()
            .map(|m| Metadata {
                key: m.key,
                value: m.value,
            })
            .collect::<Vec<Metadata>>();

        Ok(metadata)
    }

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

        let aggregation_type = match metric.aggregation_type {
            domain::enums::BillingMetricAggregateEnum::Count => AggregationType::Count,
            domain::enums::BillingMetricAggregateEnum::Latest => AggregationType::Latest,
            domain::enums::BillingMetricAggregateEnum::Max => AggregationType::Max,
            domain::enums::BillingMetricAggregateEnum::Min => AggregationType::Min,
            domain::enums::BillingMetricAggregateEnum::Mean => AggregationType::Mean,
            domain::enums::BillingMetricAggregateEnum::Sum => AggregationType::Sum,
            domain::enums::BillingMetricAggregateEnum::CountDistinct => {
                AggregationType::CountDistinct
            }
        } as i32;

        // Build segmentation filter based on the metric's segmentation matrix
        let segmentation_filter = match metric.segmentation_matrix.clone() {
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
        };

        let request = QueryMeterRequest {
            tenant_id: tenant_id.as_proto(),
            meter_slug: metric.id.to_string(),
            code: metric.code.clone(),
            meter_aggregation_type: aggregation_type,
            customer_ids: vec![customer_id.to_string()],
            from: Some(date_to_timestamp(period.start)),
            to: Some(date_to_timestamp(period.end)), // exclusive TODO check
            group_by_properties: metric
                .usage_group_key
                .as_ref()
                .map(|k| vec![k.clone()])
                .unwrap_or_default(),
            window_size: QueryWindowSize::AggregateAll.into(),
            timezone: None,
            segmentation_filter,
            value_property: metric.aggregation_key.clone(),
        };

        let mut metering_client_mut = self.usage_grpc_client.clone();
        let response: QueryMeterResponse = metering_client_mut
            .query_meter(request)
            .await
            .change_context(StoreError::MeteringServiceError)
            .attach("Failed to query meter")?
            .into_inner();

        let data: Vec<GroupedUsageData> = response
            .usage
            .into_iter()
            .map(|usage| {
                let value: Decimal = usage.value.and_then(|v| v.try_into().ok()).unwrap();

                GroupedUsageData {
                    value,
                    dimensions: usage
                        .dimensions
                        .into_iter()
                        .map(|(k, v)| (k, v.value.unwrap_or(String::new())))
                        .collect(),
                }
            })
            .collect();

        Ok(UsageData { data, period })
    }

    async fn ingest_events_from_csv(
        &self,
        tenant_id: &TenantId,
        file_data: &[u8],
        options: CsvIngestionOptions,
    ) -> StoreResult<CsvIngestionResult> {
        if file_data.is_empty() {
            bail!(StoreError::InvalidArgument("File is empty".to_string()));
        }

        if file_data.len() > MAX_CSV_SIZE {
            bail!(StoreError::InvalidArgument(format!(
                "File size exceeds maximum allowed ({MAX_CSV_SIZE} bytes)"
            )));
        }

        let delimiter = options.delimiter as u8;
        let allow_backfilling = options.allow_backfilling;
        let fail_on_error = options.fail_on_error;

        // Parse CSV (headers are required)
        let mut reader = ReaderBuilder::new()
            .has_headers(true)
            .delimiter(delimiter)
            .from_reader(file_data.reader());

        let headers = reader
            .headers()
            .change_context(StoreError::InvalidArgument(
                "Failed to read CSV headers".to_string(),
            ))?
            .clone();

        // Find required column indices
        let event_id_idx = headers.iter().position(|h| h == "event_id");
        let event_code_idx = headers.iter().position(|h| h == "event_code");
        let customer_id_idx = headers
            .iter()
            .position(|h| h == "customer_id" || h == "customer_alias");
        let timestamp_idx = headers.iter().position(|h| h == "timestamp");

        if event_code_idx.is_none() || customer_id_idx.is_none() {
            bail!(StoreError::InvalidArgument(
                "CSV must contain 'event_code' and 'customer_id' (or 'customer_alias') columns"
                    .to_string()
            ));
        }

        let mut events = Vec::new();
        let mut failures = Vec::new();
        let mut row_number = 2; // Account for header row (always present)

        for result in reader.records() {
            let record = match result {
                Ok(r) => r,
                Err(e) => {
                    failures.push(CsvIngestionFailure {
                        row_number,
                        event_id: String::new(),
                        reason: format!("Failed to parse row: {e}"),
                    });
                    row_number += 1;
                    continue;
                }
            };

            // Extract fields
            let event_id = event_id_idx.and_then(|idx| record.get(idx)).map_or_else(
                || uuid::Uuid::new_v4().to_string(),
                std::string::ToString::to_string,
            );

            let event_code = match event_code_idx.and_then(|idx| record.get(idx)) {
                Some(code) if !code.is_empty() => code.to_string(),
                _ => {
                    failures.push(CsvIngestionFailure {
                        row_number,
                        event_id: event_id.clone(),
                        reason: "Event code is required and cannot be empty".to_string(),
                    });
                    row_number += 1;
                    continue;
                }
            };

            let customer_id_str = match customer_id_idx.and_then(|idx| record.get(idx)) {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => {
                    failures.push(CsvIngestionFailure {
                        row_number,
                        event_id: event_id.clone(),
                        reason: "Customer ID is required and cannot be empty".to_string(),
                    });
                    row_number += 1;
                    continue;
                }
            };

            let customer_id: Option<EventCustomerId> =
                match AliasOr::<CustomerId>::from_str(&customer_id_str) {
                    Ok(AliasOr::Id(id)) => Some(EventCustomerId::MeteroidCustomerId(id.as_proto())),
                    Ok(AliasOr::Alias(alias)) => {
                        Some(EventCustomerId::ExternalCustomerAlias(alias))
                    }
                    Err(_) => {
                        failures.push(CsvIngestionFailure {
                            row_number,
                            event_id: event_id.clone(),
                            reason: "Customer ID must be a valid UUID or alias".to_string(),
                        });
                        row_number += 1;
                        continue;
                    }
                };

            let timestamp = timestamp_idx.and_then(|idx| record.get(idx)).map_or_else(
                || chrono::Utc::now().to_rfc3339(),
                std::string::ToString::to_string,
            );

            // Collect additional fields as properties
            let mut properties = HashMap::new();
            for (idx, value) in record.iter().enumerate() {
                let header_name = headers
                    .get(idx)
                    .map_or_else(|| format!("column_{idx}"), std::string::ToString::to_string);

                // Skip the main fields we already extracted
                if Some(idx) == event_id_idx
                    || Some(idx) == event_code_idx
                    || Some(idx) == customer_id_idx
                    || Some(idx) == timestamp_idx
                {
                    continue;
                }

                if !value.is_empty() {
                    properties.insert(header_name, value.to_string());
                }
            }

            events.push(Event {
                id: event_id,
                code: event_code,
                customer_id,
                timestamp,
                properties,
            });

            row_number += 1;
        }

        let total_rows = row_number - 2; // Subtract header and 1-based index

        // If fail_on_error is true and we have parsing failures, return error immediately
        if fail_on_error && !failures.is_empty() {
            return Ok(CsvIngestionResult {
                total_rows,
                successful_events: 0,
                failures,
            });
        }

        // Send events to metering service in batches
        let mut total_successful = 0;

        tracing::info!(
            "Processing {} events in {} batches",
            events.len(),
            events.len().div_ceil(MAX_BATCH_SIZE)
        );

        for (batch_idx, chunk) in events.chunks(MAX_BATCH_SIZE).enumerate() {
            tracing::info!(
                "Processing batch {} with {} events",
                batch_idx + 1,
                chunk.len()
            );
            let request = InternalIngestRequest {
                tenant_id: tenant_id.to_string(),
                events: chunk.to_vec(),
                allow_backfilling,
                fail_on_error,
            };

            let result = self
                .ingest_grpc_service
                .clone()
                .ingest_internal(request)
                .await;

            match result {
                Ok(response) => {
                    let ingestion_failures = response.into_inner().failures;
                    total_successful += chunk.len() - ingestion_failures.len();

                    tracing::info!(
                        "Batch {} succeeded with {} failures out of {} events",
                        batch_idx + 1,
                        ingestion_failures.len(),
                        chunk.len()
                    );

                    // Map the metering failures back to CSV row numbers
                    for failure in ingestion_failures {
                        // Find the original row number for this event
                        let original_row = events
                            .iter()
                            .position(|e| e.id == failure.event_id)
                            .map(|idx| idx + 2); // Account for header row

                        failures.push(CsvIngestionFailure {
                            row_number: original_row.unwrap_or(0) as i32,
                            event_id: failure.event_id,
                            reason: failure.reason,
                        });
                    }
                }
                Err(e) => {
                    tracing::error!("Batch {} failed entirely: {}", batch_idx + 1, e);
                    let clean_error = extract_error_message(&e);
                    // All events in this batch failed
                    for event in chunk {
                        let original_row = events
                            .iter()
                            .position(|e| e.id == event.id)
                            .map(|idx| idx + 2); // Account for header row

                        failures.push(CsvIngestionFailure {
                            row_number: original_row.unwrap_or(0) as i32,
                            event_id: event.id.clone(),
                            reason: clean_error.clone(),
                        });
                    }
                }
            }
        }

        // If fail_on_error is true and we have any failures (parsing or ingestion), return with 0 successful
        if fail_on_error && !failures.is_empty() {
            return Ok(CsvIngestionResult {
                total_rows,
                successful_events: 0,
                failures,
            });
        }

        Ok(CsvIngestionResult {
            total_rows,
            successful_events: total_successful as i32,
            failures,
        })
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

        let response = self
            .usage_grpc_client
            .clone()
            .query_raw_events(metering_request)
            .await
            .change_context(StoreError::MeteringServiceError)
            .attach("Failed to search events")?;

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
            fail_on_error: true,
        };

        let response = self
            .ingest_grpc_service
            .clone()
            .ingest_internal(grpc_request)
            .await
            .change_context(StoreError::MeteringServiceError)
            .attach("Failed to ingest events")?;

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
