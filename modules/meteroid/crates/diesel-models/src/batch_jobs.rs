use chrono::NaiveDateTime;

use crate::enums::{BatchJobChunkStatusEnum, BatchJobStatusEnum, BatchJobTypeEnum};
use common_domain::ids::{BatchJobChunkId, BatchJobId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, QueryableByName, Selectable};
use uuid::Uuid;

// ============================================================================
// batch_job
// ============================================================================

#[derive(Debug, Clone, Queryable, QueryableByName, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::batch_job)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BatchJobRow {
    pub id: BatchJobId,
    pub tenant_id: TenantId,
    pub job_type: BatchJobTypeEnum,
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

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::batch_job)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BatchJobRowNew {
    pub id: BatchJobId,
    pub tenant_id: TenantId,
    pub job_type: BatchJobTypeEnum,
    pub status: BatchJobStatusEnum,
    pub input_source_key: Option<String>,
    pub input_params: Option<serde_json::Value>,
    pub file_hash: Option<String>,
    pub created_by: Uuid,
    pub input_file_name: Option<String>,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = crate::schema::batch_job)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BatchJobRowUpdate {
    pub status: Option<BatchJobStatusEnum>,
    pub total_items: Option<i32>,
    pub processed_items: Option<i32>,
    pub failed_items: Option<i32>,
    pub locked_at: Option<Option<NaiveDateTime>>,
    pub updated_at: NaiveDateTime,
    pub completed_at: Option<Option<NaiveDateTime>>,
    pub error_message: Option<Option<String>>,
}

// ============================================================================
// batch_job_chunk
// ============================================================================

#[derive(Debug, Clone, Queryable, QueryableByName, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::batch_job_chunk)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BatchJobChunkRow {
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
    pub events: serde_json::Value,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::batch_job_chunk)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BatchJobChunkRowNew {
    pub id: BatchJobChunkId,
    pub job_id: BatchJobId,
    pub tenant_id: TenantId,
    pub chunk_index: i32,
    pub status: BatchJobChunkStatusEnum,
    pub item_offset: i32,
    pub item_count: i32,
    pub max_retries: i32,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = crate::schema::batch_job_chunk)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BatchJobChunkRowUpdate {
    pub status: Option<BatchJobChunkStatusEnum>,
    pub processed_count: Option<i32>,
    pub failed_count: Option<i32>,
    pub retry_count: Option<i32>,
    pub locked_at: Option<Option<NaiveDateTime>>,
    pub retry_after: Option<Option<NaiveDateTime>>,
    pub updated_at: NaiveDateTime,
}

// ============================================================================
// batch_job_item_failure
// ============================================================================

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::batch_job_item_failure)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BatchJobItemFailureRow {
    pub id: Uuid,
    pub chunk_id: BatchJobChunkId,
    pub job_id: BatchJobId,
    pub tenant_id: TenantId,
    pub item_index: i32,
    pub item_identifier: Option<String>,
    pub reason: String,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::batch_job_item_failure)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BatchJobItemFailureRowNew {
    pub chunk_id: BatchJobChunkId,
    pub job_id: BatchJobId,
    pub tenant_id: TenantId,
    pub item_index: i32,
    pub item_identifier: Option<String>,
    pub reason: String,
}

// ============================================================================
// batch_job_entity
// ============================================================================

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::batch_job_entity)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BatchJobEntityRow {
    pub id: Uuid,
    pub batch_job_id: BatchJobId,
    pub tenant_id: TenantId,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::batch_job_entity)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BatchJobEntityRowNew {
    pub batch_job_id: BatchJobId,
    pub tenant_id: TenantId,
    pub entity_type: String,
    pub entity_id: Uuid,
}
