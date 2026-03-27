use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use common_domain::ids::{AliasOr, CustomerId};
use csv::ReaderBuilder;
use metering_grpc::meteroid::metering::v1::Event;
use metering_grpc::meteroid::metering::v1::event::CustomerId as EventCustomerId;
use meteroid_store::clients::usage::UsageClient;
use meteroid_store::domain::batch_jobs::{BatchJob, BatchJobChunk};
use serde::Deserialize;

use crate::services::csv_ingest::normalize_csv_encoding;
use crate::workers::batch_jobs::engine::{
    BatchJobProcessor, ChunkDefinition, ChunkResult, ItemFailure,
};

const CHUNK_SIZE: i32 = 500;

pub struct EventCsvProcessor {
    usage_client: Arc<dyn UsageClient>,
}

impl EventCsvProcessor {
    pub fn new(usage_client: Arc<dyn UsageClient>) -> Self {
        Self { usage_client }
    }
}

fn deserialize_string_bool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    match &v {
        serde_json::Value::Bool(b) => Ok(*b),
        serde_json::Value::String(s) => match s.as_str() {
            "true" | "1" => Ok(true),
            _ => Ok(false),
        },
        _ => Ok(false),
    }
}

#[derive(serde::Deserialize)]
pub struct CsvInputParams {
    pub delimiter: char,
    #[serde(default, deserialize_with = "deserialize_string_bool")]
    pub fail_on_error: bool,
}

pub fn parse_input_params(job: &BatchJob) -> Result<CsvInputParams, String> {
    let params = job.input_params.as_ref().ok_or("Missing input_params")?;

    serde_json::from_value(params.clone()).map_err(|e| format!("Invalid input_params: {e}"))
}

fn build_csv_reader(data: &[u8], delimiter: u8) -> csv::Reader<&[u8]> {
    ReaderBuilder::new()
        .has_headers(true)
        .delimiter(delimiter)
        .from_reader(data)
}

/// Column indices for the known CSV columns.
struct ColumnMap {
    event_id: Option<usize>,
    event_code: Option<usize>,
    customer_id: Option<usize>,
    timestamp: Option<usize>,
}

impl ColumnMap {
    fn from_headers(headers: &csv::StringRecord) -> Result<Self, String> {
        let event_code = headers.iter().position(|h| h == "event_code");
        let customer_id = headers
            .iter()
            .position(|h| h == "customer_id" || h == "customer_alias");

        if event_code.is_none() || customer_id.is_none() {
            return Err(
                "CSV must contain 'event_code' and 'customer_id' (or 'customer_alias') columns"
                    .to_string(),
            );
        }

        Ok(Self {
            event_id: headers.iter().position(|h| h == "event_id"),
            event_code,
            customer_id,
            timestamp: headers.iter().position(|h| h == "timestamp"),
        })
    }
}

/// Parse a single CSV record into an Event, returning Err with the failure reason on bad data.
fn parse_row(
    record: &csv::StringRecord,
    headers: &csv::StringRecord,
    cols: &ColumnMap,
) -> Result<Event, String> {
    let event_id = cols
        .event_id
        .and_then(|idx| record.get(idx))
        .filter(|s| !s.is_empty())
        .map_or_else(|| uuid::Uuid::new_v4().to_string(), |s| s.to_string());

    let event_code = cols
        .event_code
        .and_then(|idx| record.get(idx))
        .filter(|s| !s.is_empty())
        .ok_or("Event code is required and cannot be empty")?
        .to_string();

    let customer_id_str = cols
        .customer_id
        .and_then(|idx| record.get(idx))
        .filter(|s| !s.is_empty())
        .ok_or("Customer ID is required and cannot be empty")?;

    let customer_id = match AliasOr::<CustomerId>::from_str(customer_id_str) {
        Ok(AliasOr::Id(id)) => Some(EventCustomerId::MeteroidCustomerId(id.as_proto())),
        Ok(AliasOr::Alias(alias)) => Some(EventCustomerId::ExternalCustomerAlias(alias)),
        Err(_) => return Err("Customer ID must be a valid UUID or alias".to_string()),
    };

    let timestamp = cols
        .timestamp
        .and_then(|idx| record.get(idx))
        .filter(|s| !s.is_empty())
        .map_or_else(|| chrono::Utc::now().to_rfc3339(), |s| s.to_string());

    let mut properties = HashMap::new();
    for (idx, value) in record.iter().enumerate() {
        if Some(idx) == cols.event_id
            || Some(idx) == cols.event_code
            || Some(idx) == cols.customer_id
            || Some(idx) == cols.timestamp
        {
            continue;
        }
        if !value.is_empty() {
            let header_name = headers
                .get(idx)
                .map_or_else(|| format!("column_{idx}"), |h| h.to_string());
            if header_name == "_error" {
                continue;
            }
            properties.insert(header_name, value.to_string());
        }
    }

    Ok(Event {
        id: event_id,
        code: event_code,
        customer_id,
        timestamp,
        properties,
    })
}

#[async_trait]
impl BatchJobProcessor for EventCsvProcessor {
    async fn prepare_chunks(
        &self,
        job: &BatchJob,
        input_data: Option<Bytes>,
    ) -> Result<Vec<ChunkDefinition>, String> {
        let data = input_data.ok_or("No input data provided for CSV job")?;
        let params = parse_input_params(job)?;
        let normalized = normalize_csv_encoding(&data);
        let delimiter = params.delimiter as u8;

        let mut reader = build_csv_reader(&normalized, delimiter);

        // Validate headers
        let headers = reader
            .headers()
            .map_err(|e| format!("Failed to read CSV headers: {e}"))?
            .clone();

        ColumnMap::from_headers(&headers)?;

        let row_count = reader.records().count() as i32;
        if row_count == 0 {
            return Err("CSV file contains no data rows".to_string());
        }

        let mut chunks = Vec::new();
        let mut offset = 0;
        while offset < row_count {
            let count = (row_count - offset).min(CHUNK_SIZE);
            chunks.push(ChunkDefinition {
                item_offset: offset,
                item_count: count,
            });
            offset += count;
        }

        Ok(chunks)
    }

    async fn process_chunk(
        &self,
        job: &BatchJob,
        chunk: &BatchJobChunk,
        input_data: Option<Bytes>,
    ) -> Result<ChunkResult, String> {
        let data = input_data.ok_or("No input data provided for CSV chunk")?;
        let params = parse_input_params(job)?;
        let normalized = normalize_csv_encoding(&data);
        let delimiter = params.delimiter as u8;

        let mut reader = build_csv_reader(&normalized, delimiter);

        let headers = reader
            .headers()
            .map_err(|e| format!("Failed to read CSV headers: {e}"))?
            .clone();

        let cols = ColumnMap::from_headers(&headers)?;

        let offset = chunk.item_offset as usize;
        let count = chunk.item_count as usize;

        let mut events = Vec::with_capacity(count);
        let mut failures = Vec::new();

        for (local_idx, result) in reader.records().enumerate().skip(offset).take(count) {
            let global_idx = local_idx as i32;

            let record = match result {
                Ok(r) => r,
                Err(e) => {
                    failures.push(ItemFailure {
                        item_index: global_idx,
                        item_identifier: None,
                        reason: format!("Failed to parse row: {e}"),
                    });
                    continue;
                }
            };

            match parse_row(&record, &headers, &cols) {
                Ok(event) => events.push(event),
                Err(reason) => {
                    let event_id = cols
                        .event_id
                        .and_then(|idx| record.get(idx))
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string());

                    failures.push(ItemFailure {
                        item_index: global_idx,
                        item_identifier: event_id,
                        reason,
                    });
                }
            }
        }

        // If fail_on_error and we already have parse failures, return early
        if params.fail_on_error && !failures.is_empty() {
            return Ok(ChunkResult {
                processed: 0,
                failures,
                created_entities: vec![],
            });
        }

        if events.is_empty() {
            return Ok(ChunkResult {
                processed: 0,
                failures,
                created_entities: vec![],
            });
        }

        let request = meteroid_store::clients::usage::IngestEventsRequest {
            events: events.clone(),
            allow_backfilling: true,
            fail_on_error: params.fail_on_error,
        };

        match self
            .usage_client
            .ingest_events(&job.tenant_id, request)
            .await
        {
            Ok(result) => {
                let ingestion_failures = result.failures;
                let ingestion_fail_count = ingestion_failures.len();
                let successful = events.len() - ingestion_fail_count;

                for failure in ingestion_failures {
                    let item_index = events
                        .iter()
                        .position(|e| e.id == failure.event_id)
                        .map(|idx| (offset + idx) as i32)
                        .unwrap_or(-1);

                    failures.push(ItemFailure {
                        item_index,
                        item_identifier: Some(failure.event_id),
                        reason: failure.reason,
                    });
                }

                if params.fail_on_error && !failures.is_empty() {
                    return Ok(ChunkResult {
                        processed: 0,
                        failures,
                        created_entities: vec![],
                    });
                }

                Ok(ChunkResult {
                    processed: successful as i32,
                    failures,
                    created_entities: vec![],
                })
            }
            Err(e) => {
                // Propagate as chunk-level error to trigger auto-retry with backoff.
                // Extract the meaningful error message (not the full error_stack trace).
                let msg = e.current_context().to_string();
                return Err(msg);
            }
        }
    }

    fn max_retries(&self) -> i32 {
        3
    }

    fn chunk_size(&self) -> i32 {
        CHUNK_SIZE
    }
}
