use chrono::NaiveDateTime;
use common_domain::ids::{BatchJobChunkId, BatchJobId, TenantId};
use diesel_models::batch_jobs::{BatchJobChunkRow, BatchJobItemFailureRow, BatchJobRow};
use o2o::o2o;
use uuid::Uuid;

use crate::domain::enums::{BatchJobChunkStatusEnum, BatchJobStatusEnum, BatchJobTypeEnum};

#[derive(Debug, Clone, o2o)]
#[from_owned(BatchJobRow)]
pub struct BatchJob {
    pub id: BatchJobId,
    pub tenant_id: TenantId,
    #[map(~.into())]
    pub job_type: BatchJobTypeEnum,
    #[map(~.into())]
    pub status: BatchJobStatusEnum,
    pub input_source_key: Option<String>,
    pub input_params: Option<serde_json::Value>,
    pub total_items: Option<i32>,
    pub processed_items: i32,
    pub failed_items: i32,
    pub file_hash: Option<String>,
    pub locked_at: Option<NaiveDateTime>,
    pub created_by: Uuid,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub completed_at: Option<NaiveDateTime>,
    pub error_message: Option<String>,
    pub error_output_key: Option<String>,
    pub input_file_name: Option<String>,
}

/// A chunk event entry stored in the JSONB `events` column.
/// Timestamps come from PostgreSQL's `NOW()::TEXT` (`"2026-03-26 17:22:46.168526+00"`).
/// We parse the timezone offset and convert to naive UTC (matching the project convention).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChunkEvent {
    pub event: String,
    pub attempt: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(deserialize_with = "deserialize_pg_timestamp_to_naive_utc")]
    pub timestamp: NaiveDateTime,
}

fn deserialize_pg_timestamp_to_naive_utc<'de, D>(deserializer: D) -> Result<NaiveDateTime, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = serde::Deserialize::deserialize(deserializer)?;
    chrono::DateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S%.f%#z")
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(&s))
        .map(|dt| dt.naive_utc())
        .map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone)]
pub struct BatchJobChunk {
    pub id: BatchJobChunkId,
    pub job_id: BatchJobId,
    pub tenant_id: TenantId,
    pub chunk_index: i32,
    pub status: BatchJobChunkStatusEnum,
    pub item_offset: i32,
    pub item_count: i32,
    pub processed_count: i32,
    pub failed_count: i32,
    pub retry_count: i32,
    pub max_retries: i32,
    pub locked_at: Option<NaiveDateTime>,
    pub retry_after: Option<NaiveDateTime>,
    pub events: Vec<ChunkEvent>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl From<BatchJobChunkRow> for BatchJobChunk {
    fn from(row: BatchJobChunkRow) -> Self {
        let events: Vec<ChunkEvent> = match serde_json::from_value(row.events.clone()) {
            Ok(v) => v,
            Err(e) => {
                log::error!(
                    "Failed to deserialize chunk events for chunk {}: {e}. Raw JSON: {}",
                    row.id,
                    row.events
                );
                vec![]
            }
        };

        Self {
            id: row.id,
            job_id: row.job_id,
            tenant_id: row.tenant_id,
            chunk_index: row.chunk_index,
            status: row.status.into(),
            item_offset: row.item_offset,
            item_count: row.item_count,
            processed_count: row.processed_count,
            failed_count: row.failed_count,
            retry_count: row.retry_count,
            max_retries: row.max_retries,
            locked_at: row.locked_at,
            retry_after: row.retry_after,
            events,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, o2o)]
#[from_owned(BatchJobItemFailureRow)]
pub struct BatchJobItemFailure {
    pub id: Uuid,
    pub chunk_id: BatchJobChunkId,
    pub job_id: BatchJobId,
    pub tenant_id: TenantId,
    pub item_index: i32,
    pub item_identifier: Option<String>,
    pub reason: String,
    pub created_at: NaiveDateTime,
}

/// Full job detail with chunks and failure count.
#[derive(Debug, Clone)]
pub struct BatchJobDetail {
    pub job: BatchJob,
    pub chunks: Vec<BatchJobChunk>,
    pub failure_count: i64,
}

/// Input for recording an item failure.
#[derive(Debug, Clone)]
pub struct BatchJobItemFailureInput {
    pub item_index: i32,
    pub item_identifier: Option<String>,
    pub reason: String,
}

/// Parameters for creating a new batch job.
#[derive(Debug)]
pub struct BatchJobNew {
    pub tenant_id: TenantId,
    pub job_type: BatchJobTypeEnum,
    pub input_source_key: Option<String>,
    pub input_params: Option<serde_json::Value>,
    pub file_hash: Option<String>,
    pub created_by: Uuid,
    pub input_file_name: Option<String>,
}

/// Input for recording entities created by a batch job.
#[derive(Debug, Clone)]
pub struct BatchJobEntityNew {
    pub batch_job_id: BatchJobId,
    pub tenant_id: TenantId,
    pub entity_type: String,
    pub entity_id: Uuid,
}
