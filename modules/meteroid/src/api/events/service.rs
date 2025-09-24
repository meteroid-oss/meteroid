use super::EventsServiceComponents;
use crate::api::events::error::EventsApiError;
use bytes::Buf;
use common_domain::ids::AliasOr;
use common_grpc::middleware::server::auth::RequestExt;
use csv::ReaderBuilder;
use metering_grpc::meteroid::metering::v1::{
    Event, InternalIngestRequest, QueryRawEventsRequest, event::CustomerId,
    query_raw_events_request::SortOrder as MeteringSortOrder,
};
use meteroid_grpc::meteroid::api::events::v1::{
    EventIngestionFailure, EventSummary, IngestEventsFromCsvRequest, IngestEventsFromCsvResponse,
    SearchEventsRequest, SearchEventsResponse,
    events_ingestion_service_server::EventsIngestionService, search_events_request::SortOrder,
};
use std::collections::HashMap;
use std::str::FromStr;
use tonic::{Request, Response, Status};

const MAX_CSV_SIZE: usize = 10 * 1024 * 1024; // 10MB limit
const MAX_BATCH_SIZE: usize = 500; // Max events per batch

#[tonic::async_trait]
impl EventsIngestionService for EventsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn ingest_events_from_csv(
        &self,
        request: Request<IngestEventsFromCsvRequest>,
    ) -> Result<Response<IngestEventsFromCsvResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let file_data = req
            .file
            .ok_or_else(|| Status::invalid_argument("No file provided"))?;

        if file_data.data.is_empty() {
            return Err(Status::invalid_argument("File is empty"));
        }

        if file_data.data.len() > MAX_CSV_SIZE {
            return Err(Status::invalid_argument(format!(
                "File size exceeds maximum allowed ({} bytes)",
                MAX_CSV_SIZE
            )));
        }

        let delimiter = req.delimiter.chars().next().unwrap_or(',') as u8;
        let allow_backfilling = req.allow_backfilling;
        let fail_on_error = req.fail_on_error;

        // Parse CSV (headers are required)
        let mut reader = ReaderBuilder::new()
            .has_headers(true)
            .delimiter(delimiter)
            .from_reader(file_data.data.reader());

        let headers = reader
            .headers()
            .map_err(|e| EventsApiError::CsvParsingError(e.to_string()))?
            .clone();

        // Find required column indices
        let event_id_idx = headers.iter().position(|h| h == "event_id");
        let event_code_idx = headers.iter().position(|h| h == "event_code");
        let customer_id_idx = headers
            .iter()
            .position(|h| h == "customer_id" || h == "customer_alias");
        let timestamp_idx = headers.iter().position(|h| h == "timestamp");

        if event_code_idx.is_none() || customer_id_idx.is_none() {
            return Err(Status::invalid_argument(
                "CSV must contain 'event_code' and 'customer_id' (or 'customer_alias') columns",
            ));
        }

        let mut events = Vec::new();
        let mut failures = Vec::new();
        let mut row_number = 2; // Account for header row (always present)

        for result in reader.records() {
            let record = match result {
                Ok(r) => r,
                Err(e) => {
                    failures.push(EventIngestionFailure {
                        row_number: row_number as i32,
                        event_id: String::new(),
                        reason: format!("Failed to parse row: {}", e),
                    });
                    row_number += 1;
                    continue;
                }
            };

            // Extract fields
            let event_id = event_id_idx
                .and_then(|idx| record.get(idx))
                .map(|s| s.to_string())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            let event_code = match event_code_idx.and_then(|idx| record.get(idx)) {
                Some(code) if !code.is_empty() => code.to_string(),
                _ => {
                    failures.push(EventIngestionFailure {
                        row_number: row_number as i32,
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
                    failures.push(EventIngestionFailure {
                        row_number: row_number as i32,
                        event_id: event_id.clone(),
                        reason: "Customer ID is required and cannot be empty".to_string(),
                    });
                    row_number += 1;
                    continue;
                }
            };

            let customer_id: Option<CustomerId> =
                match AliasOr::<common_domain::ids::CustomerId>::from_str(&customer_id_str) {
                    Ok(AliasOr::Id(id)) => Some(CustomerId::MeteroidCustomerId(id.as_proto())),
                    Ok(AliasOr::Alias(alias)) => Some(CustomerId::ExternalCustomerAlias(alias)),
                    Err(_) => {
                        failures.push(EventIngestionFailure {
                            row_number: row_number as i32,
                            event_id: event_id.clone(),
                            reason: "Customer ID must be a valid UUID or alias".to_string(),
                        });
                        row_number += 1;
                        continue;
                    }
                };

            let timestamp = timestamp_idx
                .and_then(|idx| record.get(idx))
                .map(|s| s.to_string())
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

            // Collect additional fields as properties
            let mut properties = HashMap::new();
            for (idx, value) in record.iter().enumerate() {
                let header_name = headers
                    .get(idx)
                    .map(|h| h.to_string())
                    .unwrap_or_else(|| format!("column_{}", idx));

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
            return Ok(Response::new(IngestEventsFromCsvResponse {
                total_rows: total_rows as i32,
                successful_events: 0,
                failures,
            }));
        }

        // Send events to metering service in batches
        let mut total_successful = 0;

        tracing::info!(
            "Processing {} events in {} batches",
            events.len(),
            (events.len() + MAX_BATCH_SIZE - 1) / MAX_BATCH_SIZE
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
                .metering_internal_client
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

                        failures.push(EventIngestionFailure {
                            row_number: original_row.unwrap_or(0) as i32,
                            event_id: failure.event_id,
                            reason: failure.reason,
                        });
                    }
                }
                Err(e) => {
                    tracing::error!("Batch {} failed entirely: {}", batch_idx + 1, e);
                    // All events in this batch failed
                    for event in chunk {
                        let original_row = events
                            .iter()
                            .position(|e| e.id == event.id)
                            .map(|idx| idx + 2); // Account for header row

                        failures.push(EventIngestionFailure {
                            row_number: original_row.unwrap_or(0) as i32,
                            event_id: event.id.clone(),
                            reason: format!("Metering service error: {}", e),
                        });
                    }
                }
            }
        }

        if fail_on_error && !failures.is_empty() {
            return Ok(Response::new(IngestEventsFromCsvResponse {
                total_rows: total_rows as i32,
                successful_events: 0,
                failures,
            }));
        }

        Ok(Response::new(IngestEventsFromCsvResponse {
            total_rows: total_rows as i32,
            successful_events: total_successful as i32,
            failures,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn search_events(
        &self,
        request: Request<SearchEventsRequest>,
    ) -> Result<Response<SearchEventsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let sort_order = match req.sort_order() {
            SortOrder::TimestampDesc => MeteringSortOrder::TimestampDesc,
            SortOrder::TimestampAsc => MeteringSortOrder::TimestampAsc,
            SortOrder::IngestedDesc => MeteringSortOrder::IngestedDesc,
            SortOrder::IngestedAsc => MeteringSortOrder::IngestedAsc,
        };

        let metering_request = QueryRawEventsRequest {
            tenant_id: tenant_id.to_string(),
            from: req.from,
            to: req.to,
            limit: req.limit.min(1000), // Cap at 1000
            offset: req.offset,
            search: req.search,
            event_codes: req.event_codes,
            customer_ids: req.customer_ids,
            sort_order: sort_order.into(),
        };

        match self
            .metering_query_client
            .clone()
            .query_raw_events(metering_request)
            .await
        {
            Ok(response) => {
                let metering_response = response.into_inner();

                let events = metering_response
                    .events
                    .into_iter()
                    .map(|event| EventSummary {
                        id: event.id,
                        code: event.code,
                        customer_id: match event.customer_id {
                            Some(CustomerId::MeteroidCustomerId(id)) => id,
                            Some(CustomerId::ExternalCustomerAlias(alias)) => alias,
                            None => "unknown".to_string(),
                        },
                        timestamp: event
                            .timestamp
                            .parse::<chrono::DateTime<chrono::Utc>>()
                            .map(|dt| prost_types::Timestamp {
                                seconds: dt.timestamp(),
                                nanos: dt.timestamp_subsec_nanos() as i32,
                            })
                            .ok(),
                        ingested_at: None, // TODO: Add ingested_at to Event proto
                        properties: event.properties,
                    })
                    .collect();

                Ok(Response::new(SearchEventsResponse { events }))
            }
            Err(e) => Err(Status::internal(format!("Failed to search events: {}", e))),
        }
    }
}
