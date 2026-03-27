use chrono::{NaiveDateTime, Utc};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

use crate::batch_jobs::{
    BatchJobChunkRow, BatchJobChunkRowNew, BatchJobEntityRowNew, BatchJobItemFailureRow,
    BatchJobItemFailureRowNew, BatchJobRow, BatchJobRowNew,
};
use crate::enums::{BatchJobChunkStatusEnum, BatchJobStatusEnum, BatchJobTypeEnum};
use crate::errors::IntoDbResult;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::{DbResult, PgConn};
use common_domain::ids::{BatchJobChunkId, BatchJobId, TenantId};

#[derive(diesel::QueryableByName, Debug)]
struct StalledJobIdRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub id: BatchJobId,
}

// ============================================================================
// BatchJobRowNew
// ============================================================================

impl BatchJobRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<BatchJobRow> {
        use crate::schema::batch_job::dsl::batch_job;

        let query = diesel::insert_into(batch_job).values(self);
        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting batch job")
            .into_db_result()
    }
}

// ============================================================================
// BatchJobRow
// ============================================================================

impl BatchJobRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        job_id: BatchJobId,
        tenant_id_param: TenantId,
    ) -> DbResult<BatchJobRow> {
        use crate::schema::batch_job::dsl::{batch_job, id, tenant_id};

        let query = batch_job
            .filter(id.eq(job_id))
            .filter(tenant_id.eq(tenant_id_param));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while fetching batch job by id")
            .into_db_result()
    }

    /// Worker-only: find by ID without tenant scoping.
    pub async fn find_by_id_unscoped(
        conn: &mut PgConn,
        job_id: BatchJobId,
    ) -> DbResult<BatchJobRow> {
        use crate::schema::batch_job::dsl::{batch_job, id};

        batch_job
            .filter(id.eq(job_id))
            .get_result(conn)
            .await
            .attach("Error while fetching batch job by id (unscoped)")
            .into_db_result()
    }

    pub async fn list_by_tenant(
        conn: &mut PgConn,
        tenant_id_param: TenantId,
        job_type_filter: Option<BatchJobTypeEnum>,
        status_filter: Option<Vec<BatchJobStatusEnum>>,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<BatchJobRow>> {
        use crate::schema::batch_job::dsl::{batch_job, created_at, job_type, status, tenant_id};

        let mut query = batch_job
            .filter(tenant_id.eq(tenant_id_param))
            .order_by(created_at.desc())
            .into_boxed();

        if let Some(jt) = job_type_filter {
            query = query.filter(job_type.eq(jt));
        }
        if let Some(statuses) = status_filter {
            query = query.filter(status.eq_any(statuses));
        }

        let paginated_query = query.select(BatchJobRow::as_select()).paginate(pagination);

        log::debug!(
            "{}",
            diesel::debug_query::<diesel::pg::Pg, _>(&paginated_query)
        );

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while listing batch jobs")
            .into_db_result()
    }

    /// Check for duplicate job by file hash (same tenant, type, hash, non-terminal status).
    pub async fn find_active_by_file_hash(
        conn: &mut PgConn,
        tenant_id_param: TenantId,
        job_type_param: BatchJobTypeEnum,
        hash: &str,
    ) -> DbResult<Option<BatchJobRow>> {
        use crate::schema::batch_job::dsl::{batch_job, file_hash, job_type, status, tenant_id};

        let non_terminal = vec![
            BatchJobStatusEnum::Pending,
            BatchJobStatusEnum::Chunking,
            BatchJobStatusEnum::Processing,
        ];

        let query = batch_job
            .filter(tenant_id.eq(tenant_id_param))
            .filter(job_type.eq(job_type_param))
            .filter(file_hash.eq(hash))
            .filter(status.eq_any(non_terminal));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while checking for duplicate batch job")
            .into_db_result()
    }

    /// Find a completed (or completed-with-errors) job with the same file hash.
    pub async fn find_completed_by_file_hash(
        conn: &mut PgConn,
        tenant_id_param: TenantId,
        job_type_param: BatchJobTypeEnum,
        hash: &str,
    ) -> DbResult<Option<BatchJobRow>> {
        use crate::schema::batch_job::dsl::{batch_job, file_hash, job_type, status, tenant_id};

        let completed = vec![
            BatchJobStatusEnum::Completed,
            BatchJobStatusEnum::CompletedWithErrors,
        ];

        let query = batch_job
            .filter(tenant_id.eq(tenant_id_param))
            .filter(job_type.eq(job_type_param))
            .filter(file_hash.eq(hash))
            .filter(status.eq_any(completed))
            .order(crate::schema::batch_job::dsl::created_at.desc());

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while checking for completed duplicate batch job")
            .into_db_result()
    }

    /// Atomically claim a pending job for chunking via FOR UPDATE SKIP LOCKED.
    pub async fn claim_pending_job(conn: &mut PgConn) -> DbResult<Option<BatchJobRow>> {
        let raw_sql = r#"
            UPDATE batch_job
            SET status = 'CHUNKING', locked_at = NOW(), updated_at = NOW()
            WHERE id = (
                SELECT id FROM batch_job
                WHERE status = 'PENDING'
                ORDER BY created_at ASC
                LIMIT 1
                FOR UPDATE SKIP LOCKED
            )
            RETURNING *
        "#;

        let query = diesel::sql_query(raw_sql);
        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results::<BatchJobRow>(conn)
            .await
            .attach("Error while claiming pending batch job")
            .map(|mut v| v.pop())
            .into_db_result()
    }

    /// Transition job from CHUNKING to PROCESSING after chunks are created.
    pub async fn mark_as_processing(
        conn: &mut PgConn,
        job_id: BatchJobId,
        total: i32,
    ) -> DbResult<()> {
        use crate::schema::batch_job::dsl::{
            batch_job, id, locked_at, status, total_items, updated_at,
        };

        let now = Utc::now().naive_utc();
        let query = diesel::update(batch_job)
            .filter(id.eq(job_id))
            .filter(status.eq(BatchJobStatusEnum::Chunking))
            .set((
                status.eq(BatchJobStatusEnum::Processing),
                total_items.eq(Some(total)),
                locked_at.eq(None::<chrono::NaiveDateTime>),
                updated_at.eq(now),
            ));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while marking batch job as processing")
            .map(|_| ())
            .into_db_result()
    }

    /// Finalize the job status based on chunk outcomes.
    /// Uses CAS on status=PROCESSING and computes processed/failed counts atomically via subquery.
    pub async fn finalize(
        conn: &mut PgConn,
        job_id: BatchJobId,
        final_status: BatchJobStatusEnum,
    ) -> DbResult<usize> {
        let now = Utc::now().naive_utc();
        let status_sql = match final_status {
            BatchJobStatusEnum::Completed => "COMPLETED",
            BatchJobStatusEnum::CompletedWithErrors => "COMPLETED_WITH_ERRORS",
            BatchJobStatusEnum::Failed => "FAILED",
            _ => "FAILED",
        };

        let raw_sql = format!(
            r#"
            UPDATE batch_job
            SET
                status = '{status_sql}'::"BatchJobStatusEnum",
                processed_items = (
                    SELECT COALESCE(SUM(processed_count), 0)::INT4
                    FROM batch_job_chunk WHERE job_id = $1
                ),
                failed_items = (
                    SELECT COALESCE(SUM(failed_count), 0)::INT4
                    FROM batch_job_chunk WHERE job_id = $1
                ),
                completed_at = $2,
                updated_at = $2
            WHERE id = $1 AND status = 'PROCESSING'::"BatchJobStatusEnum"
            "#
        );

        log::debug!("{}", raw_sql);

        diesel::sql_query(&raw_sql)
            .bind::<diesel::sql_types::Uuid, _>(*job_id)
            .bind::<diesel::sql_types::Timestamp, _>(now)
            .execute(conn)
            .await
            .attach("Error while finalizing batch job")
            .into_db_result()
    }

    /// Set the error_output_key (S3 key of the pre-built error CSV).
    /// Uses CAS on error_output_key IS NULL to avoid races.
    pub async fn set_error_output_key(
        conn: &mut PgConn,
        job_id: BatchJobId,
        key: String,
    ) -> DbResult<usize> {
        use crate::schema::batch_job::dsl;

        let now = Utc::now().naive_utc();
        let query = diesel::update(dsl::batch_job)
            .filter(dsl::id.eq(job_id))
            .filter(dsl::error_output_key.is_null())
            .set((dsl::error_output_key.eq(Some(key)), dsl::updated_at.eq(now)));

        query
            .execute(conn)
            .await
            .attach("Error while setting error_output_key")
            .into_db_result()
    }

    /// Mark a job as failed. Only applies to non-terminal states.
    pub async fn mark_as_failed(
        conn: &mut PgConn,
        job_id: BatchJobId,
        error: Option<String>,
    ) -> DbResult<()> {
        use crate::schema::batch_job::dsl::{
            batch_job, completed_at, error_message, id, locked_at, status, updated_at,
        };

        let now = Utc::now().naive_utc();
        let query = diesel::update(batch_job)
            .filter(id.eq(job_id))
            .filter(
                status
                    .eq(BatchJobStatusEnum::Pending)
                    .or(status.eq(BatchJobStatusEnum::Chunking))
                    .or(status.eq(BatchJobStatusEnum::Processing)),
            )
            .set((
                status.eq(BatchJobStatusEnum::Failed),
                locked_at.eq(None::<chrono::NaiveDateTime>),
                completed_at.eq(Some(now)),
                updated_at.eq(now),
                error_message.eq(error),
            ));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while marking batch job as failed")
            .map(|_| ())
            .into_db_result()
    }

    /// Cancel a job and all its pending chunks.
    pub async fn cancel(
        conn: &mut PgConn,
        job_id: BatchJobId,
        tenant_id_param: TenantId,
    ) -> DbResult<()> {
        use crate::schema::batch_job::dsl::{
            batch_job, completed_at, id, locked_at, status, tenant_id, updated_at,
        };

        let now = Utc::now().naive_utc();
        let query = diesel::update(batch_job)
            .filter(id.eq(job_id))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(
                status
                    .eq(BatchJobStatusEnum::Pending)
                    .or(status.eq(BatchJobStatusEnum::Chunking))
                    .or(status.eq(BatchJobStatusEnum::Processing)),
            )
            .set((
                status.eq(BatchJobStatusEnum::Cancelled),
                locked_at.eq(None::<chrono::NaiveDateTime>),
                completed_at.eq(Some(now)),
                updated_at.eq(now),
            ));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while cancelling batch job")
            .map(|_| ())
            .into_db_result()
    }

    /// Reopen a completed/failed job for retry by setting it back to PROCESSING.
    pub async fn reopen_for_retry(
        conn: &mut PgConn,
        job_id: BatchJobId,
        tenant_id_param: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::batch_job::dsl::{
            batch_job, completed_at, error_output_key, id, status, tenant_id, updated_at,
        };

        let now = Utc::now().naive_utc();
        let query = diesel::update(batch_job)
            .filter(id.eq(job_id))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(
                status
                    .eq(BatchJobStatusEnum::CompletedWithErrors)
                    .or(status.eq(BatchJobStatusEnum::Failed)),
            )
            .set((
                status.eq(BatchJobStatusEnum::Processing),
                completed_at.eq(None::<chrono::NaiveDateTime>),
                error_output_key.eq(None::<String>),
                updated_at.eq(now),
            ));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while reopening batch job for retry")
            .into_db_result()
    }

    /// Find PROCESSING jobs where all chunks are in terminal state and the job
    /// has been stale for at least 1 minute (avoids racing with normal finalization).
    pub async fn find_stalled_processing_jobs(conn: &mut PgConn) -> DbResult<Vec<BatchJobId>> {
        let threshold = Utc::now().naive_utc() - chrono::Duration::minutes(1);

        let raw_sql = r#"
            SELECT j.id FROM batch_job j
            WHERE j.status = 'PROCESSING'::"BatchJobStatusEnum"
            AND j.updated_at <= $1
            AND NOT EXISTS (
                SELECT 1 FROM batch_job_chunk c
                WHERE c.job_id = j.id
                AND c.status IN ('PENDING'::"BatchJobChunkStatusEnum", 'PROCESSING'::"BatchJobChunkStatusEnum")
            )
            AND EXISTS (
                SELECT 1 FROM batch_job_chunk c WHERE c.job_id = j.id
            )
            LIMIT 10
        "#;

        log::debug!("{}", raw_sql);

        diesel::sql_query(raw_sql)
            .bind::<diesel::sql_types::Timestamp, _>(threshold)
            .get_results::<StalledJobIdRow>(conn)
            .await
            .attach("Error while finding stalled processing jobs")
            .map(|rows| rows.into_iter().map(|r| r.id).collect())
            .into_db_result()
    }

    /// Fail PROCESSING jobs that have exceeded the maximum allowed age.
    /// Skips all their PENDING chunks and marks the job as FAILED.
    /// Returns the IDs of timed-out jobs so the caller can finalize them.
    pub async fn fail_timed_out_jobs(
        conn: &mut PgConn,
        max_age: chrono::Duration,
    ) -> DbResult<Vec<BatchJobId>> {
        let threshold = Utc::now().naive_utc() - max_age;

        // Skip all PENDING chunks for timed-out jobs
        let skip_sql = r#"
            UPDATE batch_job_chunk
            SET
                status = 'SKIPPED'::"BatchJobChunkStatusEnum",
                locked_at = NULL,
                updated_at = NOW(),
                events = events || jsonb_build_array(jsonb_build_object(
                    'event', 'SKIPPED',
                    'attempt', retry_count + 1,
                    'message', 'Job timed out',
                    'timestamp', NOW()::TEXT
                ))
            WHERE status = 'PENDING'::"BatchJobChunkStatusEnum"
            AND job_id IN (
                SELECT id FROM batch_job
                WHERE status = 'PROCESSING'::"BatchJobStatusEnum"
                AND created_at <= $1
            )
        "#;

        diesel::sql_query(skip_sql)
            .bind::<diesel::sql_types::Timestamp, _>(threshold)
            .execute(conn)
            .await
            .attach("Error skipping chunks for timed-out jobs")
            .into_db_result()?;

        // Return the job IDs (stalled-job sweep will finalize them)
        let find_sql = r#"
            SELECT j.id FROM batch_job j
            WHERE j.status = 'PROCESSING'::"BatchJobStatusEnum"
            AND j.created_at <= $1
            AND NOT EXISTS (
                SELECT 1 FROM batch_job_chunk c
                WHERE c.job_id = j.id
                AND c.status IN ('PENDING'::"BatchJobChunkStatusEnum", 'PROCESSING'::"BatchJobChunkStatusEnum")
            )
        "#;

        diesel::sql_query(find_sql)
            .bind::<diesel::sql_types::Timestamp, _>(threshold)
            .get_results::<StalledJobIdRow>(conn)
            .await
            .attach("Error finding timed-out jobs")
            .map(|rows| rows.into_iter().map(|r| r.id).collect())
            .into_db_result()
    }

    /// Reset stuck CHUNKING jobs back to PENDING.
    pub async fn reset_stuck_chunking_jobs(conn: &mut PgConn) -> DbResult<usize> {
        use crate::schema::batch_job::dsl::{batch_job, locked_at, status, updated_at};

        let threshold = Utc::now().naive_utc() - chrono::Duration::minutes(5);

        let query = diesel::update(batch_job)
            .filter(status.eq(BatchJobStatusEnum::Chunking))
            .filter(locked_at.le(threshold))
            .set((
                status.eq(BatchJobStatusEnum::Pending),
                locked_at.eq(None::<chrono::NaiveDateTime>),
                updated_at.eq(Utc::now().naive_utc()),
            ));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while resetting stuck chunking jobs")
            .into_db_result()
    }
}

// ============================================================================
// BatchJobChunkRowNew
// ============================================================================

impl BatchJobChunkRowNew {
    pub async fn insert_batch(
        conn: &mut PgConn,
        chunks: &[BatchJobChunkRowNew],
    ) -> DbResult<Vec<BatchJobChunkRow>> {
        use crate::schema::batch_job_chunk::dsl::batch_job_chunk;

        let query = diesel::insert_into(batch_job_chunk).values(chunks);
        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while inserting batch job chunks")
            .into_db_result()
    }
}

// ============================================================================
// BatchJobChunkRow
// ============================================================================

impl BatchJobChunkRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        chunk_id: BatchJobChunkId,
        tenant_id_param: TenantId,
    ) -> DbResult<BatchJobChunkRow> {
        use crate::schema::batch_job_chunk::dsl::{batch_job_chunk, id, tenant_id};

        let query = batch_job_chunk
            .filter(id.eq(chunk_id))
            .filter(tenant_id.eq(tenant_id_param));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while fetching batch job chunk by id")
            .into_db_result()
    }

    pub async fn list_by_job(
        conn: &mut PgConn,
        job_id_param: BatchJobId,
        tenant_id_param: TenantId,
    ) -> DbResult<Vec<BatchJobChunkRow>> {
        use crate::schema::batch_job_chunk::dsl::{
            batch_job_chunk, chunk_index, job_id, tenant_id,
        };

        let query = batch_job_chunk
            .filter(job_id.eq(job_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .order_by(chunk_index.asc());

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing batch job chunks")
            .into_db_result()
    }

    /// Atomically claim a pending chunk for processing via FOR UPDATE SKIP LOCKED.
    pub async fn claim_pending_chunk(conn: &mut PgConn) -> DbResult<Option<BatchJobChunkRow>> {
        let raw_sql = r#"
            UPDATE batch_job_chunk
            SET status = 'PROCESSING', locked_at = NOW(), updated_at = NOW()
            WHERE id = (
                SELECT c.id FROM batch_job_chunk c
                INNER JOIN batch_job j ON j.id = c.job_id
                WHERE c.status = 'PENDING' AND j.status = 'PROCESSING'
                AND (c.retry_after IS NULL OR c.retry_after <= NOW())
                ORDER BY j.created_at ASC, c.chunk_index ASC
                LIMIT 1
                FOR UPDATE OF c SKIP LOCKED
            )
            RETURNING *
        "#;

        let query = diesel::sql_query(raw_sql);
        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results::<BatchJobChunkRow>(conn)
            .await
            .attach("Error while claiming pending batch job chunk")
            .map(|mut v| v.pop())
            .into_db_result()
    }

    /// Mark chunk as completed. Uses CAS on status=PROCESSING to prevent zombie workers.
    /// Atomically appends a COMPLETED event and increments the parent job's progress counters.
    /// Returns rows updated (0 = evicted, 1 = success).
    pub async fn mark_as_completed(
        conn: &mut PgConn,
        chunk_id: BatchJobChunkId,
        processed: i32,
        failed: i32,
    ) -> DbResult<usize> {
        let raw_sql = r#"
            WITH updated_chunk AS (
                UPDATE batch_job_chunk
                SET
                    status = 'COMPLETED'::"BatchJobChunkStatusEnum",
                    processed_count = $2,
                    failed_count = $3,
                    locked_at = NULL,
                    updated_at = NOW(),
                    events = events || jsonb_build_array(jsonb_build_object(
                        'event', 'COMPLETED',
                        'attempt', retry_count + 1,
                        'message', CASE WHEN $3 > 0 THEN 'Completed with ' || $3 || ' item failures' ELSE NULL END,
                        'timestamp', NOW()::TEXT
                    ))
                WHERE id = $1
                AND status = 'PROCESSING'::"BatchJobChunkStatusEnum"
                RETURNING job_id
            )
            UPDATE batch_job
            SET
                processed_items = processed_items + $2,
                failed_items = failed_items + $3,
                updated_at = NOW()
            WHERE id = (SELECT job_id FROM updated_chunk)
        "#;

        log::debug!("{}", raw_sql);

        diesel::sql_query(raw_sql)
            .bind::<diesel::sql_types::Uuid, _>(*chunk_id)
            .bind::<diesel::sql_types::Integer, _>(processed)
            .bind::<diesel::sql_types::Integer, _>(failed)
            .execute(conn)
            .await
            .attach("Error while marking chunk as completed")
            .into_db_result()
    }

    /// Mark chunk as failed (exhausted retries). Uses CAS on status=PROCESSING.
    /// Atomically appends an EXHAUSTED event and increments the parent job's failed_items
    /// by the chunk's item_count. Returns rows updated (0 = evicted, 1 = success).
    pub async fn mark_as_failed(
        conn: &mut PgConn,
        chunk_id: BatchJobChunkId,
        error_message: &str,
    ) -> DbResult<usize> {
        let raw_sql = r#"
            WITH updated_chunk AS (
                UPDATE batch_job_chunk
                SET
                    status = 'FAILED'::"BatchJobChunkStatusEnum",
                    failed_count = item_count,
                    locked_at = NULL,
                    updated_at = NOW(),
                    events = events || jsonb_build_array(jsonb_build_object(
                        'event', 'EXHAUSTED',
                        'attempt', retry_count + 1,
                        'message', $2,
                        'timestamp', NOW()::TEXT
                    ))
                WHERE id = $1
                AND status = 'PROCESSING'::"BatchJobChunkStatusEnum"
                RETURNING job_id, item_count
            )
            UPDATE batch_job
            SET
                failed_items = failed_items + (SELECT item_count FROM updated_chunk),
                updated_at = NOW()
            WHERE id = (SELECT job_id FROM updated_chunk)
        "#;

        log::debug!("{}", raw_sql);

        diesel::sql_query(raw_sql)
            .bind::<diesel::sql_types::Uuid, _>(*chunk_id)
            .bind::<diesel::sql_types::Text, _>(error_message)
            .execute(conn)
            .await
            .attach("Error while marking chunk as failed")
            .into_db_result()
    }

    /// Schedule a chunk for retry. Uses CAS on status=PROCESSING (zombie protection).
    /// Sets status to PENDING, increments retry_count, sets retry_after, and appends
    /// ERRORED + RETRYING events atomically.
    pub async fn schedule_retry(
        conn: &mut PgConn,
        chunk_id: BatchJobChunkId,
        retry_after_ts: NaiveDateTime,
        error_msg: &str,
    ) -> DbResult<usize> {
        let raw_sql = r#"
            UPDATE batch_job_chunk
            SET
                status = 'PENDING'::"BatchJobChunkStatusEnum",
                retry_count = retry_count + 1,
                retry_after = $2,
                locked_at = NULL,
                updated_at = NOW(),
                events = events || jsonb_build_array(
                    jsonb_build_object(
                        'event', 'ERRORED',
                        'attempt', retry_count + 1,
                        'message', $3,
                        'timestamp', NOW()::TEXT
                    ),
                    jsonb_build_object(
                        'event', 'RETRYING',
                        'attempt', retry_count + 1,
                        'message', 'Retrying after ' || EXTRACT(EPOCH FROM ($2 - NOW()))::INT || 's',
                        'timestamp', NOW()::TEXT
                    )
                )
            WHERE id = $1
            AND status = 'PROCESSING'::"BatchJobChunkStatusEnum"
        "#;

        log::debug!("{}", raw_sql);

        diesel::sql_query(raw_sql)
            .bind::<diesel::sql_types::Uuid, _>(*chunk_id)
            .bind::<diesel::sql_types::Timestamp, _>(retry_after_ts)
            .bind::<diesel::sql_types::Text, _>(error_msg)
            .execute(conn)
            .await
            .attach("Error while scheduling chunk retry")
            .into_db_result()
    }

    /// Append a single event to a chunk's events JSONB array (no CAS — used for STARTED etc).
    pub async fn append_event(
        conn: &mut PgConn,
        chunk_id: BatchJobChunkId,
        event_type: &str,
        attempt: i32,
        message: Option<&str>,
    ) -> DbResult<usize> {
        let raw_sql = r#"
            UPDATE batch_job_chunk
            SET
                events = events || jsonb_build_array(jsonb_build_object(
                    'event', $2,
                    'attempt', $3,
                    'message', $4,
                    'timestamp', NOW()::TEXT
                )),
                updated_at = NOW()
            WHERE id = $1
        "#;

        log::debug!("{}", raw_sql);

        diesel::sql_query(raw_sql)
            .bind::<diesel::sql_types::Uuid, _>(*chunk_id)
            .bind::<diesel::sql_types::Text, _>(event_type)
            .bind::<diesel::sql_types::Integer, _>(attempt)
            .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(message)
            .execute(conn)
            .await
            .attach("Error while appending event to chunk")
            .into_db_result()
    }

    /// Bulk-reset all failed chunks for a job back to pending.
    pub async fn retry_failed_chunks_for_job(
        conn: &mut PgConn,
        job_id_param: BatchJobId,
        tenant_id_param: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::batch_job_chunk::dsl::{
            batch_job_chunk, job_id, locked_at, retry_count, status, tenant_id, updated_at,
        };

        let now = Utc::now().naive_utc();
        let query = diesel::update(batch_job_chunk)
            .filter(job_id.eq(job_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .filter(status.eq(BatchJobChunkStatusEnum::Failed))
            .set((
                status.eq(BatchJobChunkStatusEnum::Pending),
                retry_count.eq(retry_count + 1),
                locked_at.eq(None::<chrono::NaiveDateTime>),
                updated_at.eq(now),
            ));

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while retrying failed chunks for job")
            .into_db_result()
    }

    /// Reset stuck PROCESSING chunks (locked_at older than threshold).
    /// Single atomic UPDATE: retryable chunks go to PENDING, exhausted chunks go to FAILED.
    /// Appends appropriate events (ERRORED+RETRYING for retryable, ERRORED+EXHAUSTED for exhausted).
    pub async fn reset_stuck_chunks(conn: &mut PgConn) -> DbResult<usize> {
        let threshold = Utc::now().naive_utc() - chrono::Duration::minutes(5);

        let raw_sql = r#"
            UPDATE batch_job_chunk
            SET
                status = CASE
                    WHEN retry_count < max_retries THEN 'PENDING'::"BatchJobChunkStatusEnum"
                    ELSE 'FAILED'::"BatchJobChunkStatusEnum"
                END,
                retry_count = CASE
                    WHEN retry_count < max_retries THEN retry_count + 1
                    ELSE retry_count
                END,
                locked_at = NULL,
                updated_at = NOW(),
                events = events || CASE
                    WHEN retry_count < max_retries THEN jsonb_build_array(
                        jsonb_build_object(
                            'event', 'ERRORED',
                            'attempt', retry_count + 1,
                            'message', 'Worker timed out',
                            'timestamp', NOW()::TEXT
                        ),
                        jsonb_build_object(
                            'event', 'RETRYING',
                            'attempt', retry_count + 1,
                            'message', 'Worker timed out',
                            'timestamp', NOW()::TEXT
                        )
                    )
                    ELSE jsonb_build_array(
                        jsonb_build_object(
                            'event', 'ERRORED',
                            'attempt', retry_count + 1,
                            'message', 'Worker timed out',
                            'timestamp', NOW()::TEXT
                        ),
                        jsonb_build_object(
                            'event', 'EXHAUSTED',
                            'attempt', retry_count + 1,
                            'message', 'Worker timed out',
                            'timestamp', NOW()::TEXT
                        )
                    )
                END
            WHERE status = 'PROCESSING'::"BatchJobChunkStatusEnum"
            AND locked_at <= $1
        "#;

        log::debug!("{}", raw_sql);

        diesel::sql_query(raw_sql)
            .bind::<diesel::sql_types::Timestamp, _>(threshold)
            .execute(conn)
            .await
            .attach("Error while resetting stuck chunks")
            .into_db_result()
    }

    /// Get chunk summary for a job (used for finalization check).
    /// Single query with FILTER clauses instead of 4 sequential roundtrips.
    pub async fn get_job_chunk_summary(
        conn: &mut PgConn,
        job_id_param: BatchJobId,
    ) -> DbResult<ChunkSummary> {
        let raw_sql = r#"
            SELECT
                COUNT(*)::INT4 as total_chunks,
                COUNT(*) FILTER (WHERE status IN ('PENDING'::"BatchJobChunkStatusEnum", 'PROCESSING'::"BatchJobChunkStatusEnum"))::INT4 as active_chunks,
                COUNT(*) FILTER (WHERE status = 'FAILED'::"BatchJobChunkStatusEnum")::INT4 as failed_chunks,
                COALESCE(SUM(processed_count), 0)::INT4 as total_processed,
                COALESCE(SUM(failed_count), 0)::INT4 as total_failed
            FROM batch_job_chunk
            WHERE job_id = $1
        "#;

        diesel::sql_query(raw_sql)
            .bind::<diesel::sql_types::Uuid, _>(*job_id_param)
            .get_result::<ChunkSummary>(conn)
            .await
            .attach("Error fetching job chunk summary")
            .into_db_result()
    }

    /// Cancel all pending chunks for a job. Appends a SKIPPED event to each.
    pub async fn cancel_pending_for_job(
        conn: &mut PgConn,
        job_id_param: BatchJobId,
        tenant_id_param: TenantId,
    ) -> DbResult<usize> {
        let raw_sql = r#"
            UPDATE batch_job_chunk
            SET
                status = 'SKIPPED'::"BatchJobChunkStatusEnum",
                locked_at = NULL,
                updated_at = NOW(),
                events = events || jsonb_build_array(jsonb_build_object(
                    'event', 'SKIPPED',
                    'attempt', retry_count + 1,
                    'message', 'Job cancelled',
                    'timestamp', NOW()::TEXT
                ))
            WHERE job_id = $1
            AND tenant_id = $2
            AND status = 'PENDING'::"BatchJobChunkStatusEnum"
        "#;

        log::debug!("{}", raw_sql);

        diesel::sql_query(raw_sql)
            .bind::<diesel::sql_types::Uuid, _>(*job_id_param)
            .bind::<diesel::sql_types::Uuid, _>(*tenant_id_param)
            .execute(conn)
            .await
            .attach("Error while cancelling pending chunks")
            .into_db_result()
    }
}

/// Summary of chunk statuses for a job.
#[derive(Debug, diesel::QueryableByName)]
pub struct ChunkSummary {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub total_chunks: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub active_chunks: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub failed_chunks: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub total_processed: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub total_failed: i32,
}

// ============================================================================
// BatchJobItemFailureRowNew
// ============================================================================

impl BatchJobItemFailureRowNew {
    pub async fn insert_batch(
        conn: &mut PgConn,
        failures: &[BatchJobItemFailureRowNew],
    ) -> DbResult<usize> {
        use crate::schema::batch_job_item_failure::dsl::batch_job_item_failure;

        if failures.is_empty() {
            return Ok(0);
        }

        let query = diesel::insert_into(batch_job_item_failure).values(failures);
        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while inserting batch job item failures")
            .into_db_result()
    }
}

impl BatchJobItemFailureRow {
    pub async fn list_by_job(
        conn: &mut PgConn,
        job_id_param: BatchJobId,
        tenant_id_param: TenantId,
        chunk_id_filter: Option<uuid::Uuid>,
        limit: i64,
        offset: i64,
    ) -> DbResult<Vec<BatchJobItemFailureRow>> {
        use crate::schema::batch_job_item_failure::dsl::{
            batch_job_item_failure, chunk_id, item_index, job_id, tenant_id,
        };

        let mut query = batch_job_item_failure
            .filter(job_id.eq(job_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .order_by(item_index.asc())
            .limit(limit)
            .offset(offset)
            .into_boxed();

        if let Some(cid) = chunk_id_filter {
            query = query.filter(chunk_id.eq(cid));
        }

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing batch job item failures")
            .into_db_result()
    }

    pub async fn count_by_job(
        conn: &mut PgConn,
        job_id_param: BatchJobId,
        tenant_id_param: TenantId,
        chunk_id_filter: Option<uuid::Uuid>,
    ) -> DbResult<i64> {
        use crate::schema::batch_job_item_failure::dsl::{
            batch_job_item_failure, chunk_id, job_id, tenant_id,
        };

        let mut query = batch_job_item_failure
            .filter(job_id.eq(job_id_param))
            .filter(tenant_id.eq(tenant_id_param))
            .into_boxed();

        if let Some(cid) = chunk_id_filter {
            query = query.filter(chunk_id.eq(cid));
        }

        let count_query = query.count();
        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&count_query));

        count_query
            .get_result(conn)
            .await
            .attach("Error while counting batch job item failures")
            .into_db_result()
    }

    /// Delete all failures for failed chunks of a job (used when retrying all failed chunks).
    pub async fn delete_for_failed_chunks(
        conn: &mut PgConn,
        job_id_param: BatchJobId,
        tenant_id_param: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::batch_job_item_failure::dsl::{
            batch_job_item_failure, chunk_id, job_id, tenant_id,
        };

        let failed_chunk_ids = {
            use crate::schema::batch_job_chunk::dsl::{
                batch_job_chunk, id as chunk_pk, job_id as cj_job_id, status,
            };
            batch_job_chunk
                .filter(cj_job_id.eq(job_id_param))
                .filter(status.eq(BatchJobChunkStatusEnum::Failed))
                .select(chunk_pk)
        };

        let query = diesel::delete(
            batch_job_item_failure
                .filter(job_id.eq(job_id_param))
                .filter(tenant_id.eq(tenant_id_param))
                .filter(chunk_id.eq_any(failed_chunk_ids)),
        );

        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while deleting failures for failed chunks")
            .into_db_result()
    }
}

// ============================================================================
// BatchJobEntityRowNew
// ============================================================================

impl BatchJobEntityRowNew {
    pub async fn insert_batch(
        conn: &mut PgConn,
        entities: &[BatchJobEntityRowNew],
    ) -> DbResult<usize> {
        use crate::schema::batch_job_entity::dsl::batch_job_entity;

        if entities.is_empty() {
            return Ok(0);
        }

        let query = diesel::insert_into(batch_job_entity).values(entities);
        log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while inserting batch job entities")
            .into_db_result()
    }
}
