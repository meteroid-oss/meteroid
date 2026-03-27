use crate::StoreResult;
use crate::domain::batch_jobs::{
    BatchJob, BatchJobChunk, BatchJobDetail, BatchJobEntityNew, BatchJobItemFailure,
    BatchJobItemFailureInput, BatchJobNew,
};
use crate::domain::enums::{BatchJobStatusEnum, BatchJobTypeEnum};
use crate::domain::misc::{PaginatedVec, PaginationRequest};
use crate::errors::StoreError;
use crate::store::Store;
use common_domain::ids::{BaseId, BatchJobChunkId, BatchJobId, TenantId};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::batch_jobs::{
    BatchJobChunkRowNew, BatchJobEntityRowNew, BatchJobItemFailureRow, BatchJobItemFailureRowNew,
    BatchJobRow, BatchJobRowNew,
};
use diesel_models::enums as diesel_enums;
use error_stack::Report;

#[async_trait::async_trait]
pub trait BatchJobsInterface {
    async fn create_batch_job(&self, new_job: BatchJobNew) -> StoreResult<BatchJob>;

    async fn get_batch_job(
        &self,
        job_id: BatchJobId,
        tenant_id: TenantId,
    ) -> StoreResult<BatchJobDetail>;

    async fn list_batch_jobs(
        &self,
        tenant_id: TenantId,
        job_type: Option<BatchJobTypeEnum>,
        status: Option<Vec<BatchJobStatusEnum>>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<BatchJob>>;

    async fn check_duplicate_batch_job(
        &self,
        tenant_id: TenantId,
        job_type: BatchJobTypeEnum,
        file_hash: &str,
    ) -> StoreResult<Option<BatchJob>>;

    async fn check_completed_duplicate_batch_job(
        &self,
        tenant_id: TenantId,
        job_type: BatchJobTypeEnum,
        file_hash: &str,
    ) -> StoreResult<Option<BatchJob>>;

    async fn cancel_batch_job(&self, job_id: BatchJobId, tenant_id: TenantId) -> StoreResult<()>;

    async fn retry_failed_chunks(
        &self,
        job_id: BatchJobId,
        tenant_id: TenantId,
    ) -> StoreResult<u32>;

    async fn list_batch_job_failures(
        &self,
        job_id: BatchJobId,
        tenant_id: TenantId,
        chunk_id: Option<BatchJobChunkId>,
        limit: i64,
        offset: i64,
    ) -> StoreResult<Vec<BatchJobItemFailure>>;

    async fn count_batch_job_failures(
        &self,
        job_id: BatchJobId,
        tenant_id: TenantId,
        chunk_id: Option<BatchJobChunkId>,
    ) -> StoreResult<i64>;

    // --- Worker-facing methods ---

    async fn get_batch_job_unscoped(&self, job_id: BatchJobId) -> StoreResult<BatchJob>;

    async fn claim_pending_job(&self) -> StoreResult<Option<BatchJob>>;

    async fn complete_chunking(
        &self,
        job_id: BatchJobId,
        tenant_id: TenantId,
        chunks: Vec<(i32, i32)>, // (item_offset, item_count) per chunk
        total_items: i32,
        max_retries: i32,
    ) -> StoreResult<()>;

    async fn fail_job(&self, job_id: BatchJobId, error: Option<String>) -> StoreResult<()>;

    async fn claim_pending_chunk(&self) -> StoreResult<Option<BatchJobChunk>>;

    /// Returns false if the chunk was already reclaimed (zombie worker evicted).
    async fn complete_chunk(
        &self,
        chunk_id: BatchJobChunkId,
        job_id: BatchJobId,
        tenant_id: TenantId,
        processed: i32,
        failed: i32,
        failures: Vec<BatchJobItemFailureInput>,
    ) -> StoreResult<bool>;

    /// Returns false if the chunk was already reclaimed (zombie worker evicted).
    async fn fail_chunk(&self, chunk_id: BatchJobChunkId, error_message: &str)
    -> StoreResult<bool>;

    async fn schedule_chunk_retry(
        &self,
        chunk_id: BatchJobChunkId,
        retry_after: chrono::NaiveDateTime,
        error_message: &str,
    ) -> StoreResult<bool>;

    async fn append_chunk_event(
        &self,
        chunk_id: BatchJobChunkId,
        event_type: &str,
        attempt: i32,
        message: Option<&str>,
    ) -> StoreResult<()>;

    async fn record_chunk_item_failures(
        &self,
        chunk_id: BatchJobChunkId,
        job_id: BatchJobId,
        tenant_id: TenantId,
        failures: Vec<BatchJobItemFailureInput>,
    ) -> StoreResult<()>;

    async fn try_finalize_job(&self, job_id: BatchJobId)
    -> StoreResult<Option<BatchJobStatusEnum>>;

    async fn set_error_output_key(&self, job_id: BatchJobId, key: String) -> StoreResult<()>;

    async fn list_failed_item_indices(
        &self,
        job_id: BatchJobId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<(i32, String)>>;

    async fn reset_stuck_items(&self) -> StoreResult<()>;

    async fn find_stalled_processing_jobs(&self) -> StoreResult<Vec<BatchJobId>>;

    async fn fail_timed_out_jobs(&self, max_age: chrono::Duration) -> StoreResult<Vec<BatchJobId>>;

    async fn record_batch_job_entities(&self, entities: Vec<BatchJobEntityNew>) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl BatchJobsInterface for Store {
    async fn create_batch_job(&self, new_job: BatchJobNew) -> StoreResult<BatchJob> {
        let mut conn = self.get_conn().await?;

        let row = BatchJobRowNew {
            id: BatchJobId::new(),
            tenant_id: new_job.tenant_id,
            job_type: new_job.job_type.into(),
            status: diesel_enums::BatchJobStatusEnum::Pending,
            input_source_key: new_job.input_source_key,
            input_params: new_job.input_params,
            file_hash: new_job.file_hash,
            created_by: new_job.created_by,
            input_file_name: new_job.input_file_name,
        };

        let result = row
            .insert(&mut conn)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(result.into())
    }

    async fn get_batch_job(
        &self,
        job_id: BatchJobId,
        tenant_id: TenantId,
    ) -> StoreResult<BatchJobDetail> {
        let mut conn = self.get_conn().await?;

        let job_row = BatchJobRow::find_by_id(&mut conn, job_id, tenant_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let chunk_rows =
            diesel_models::batch_jobs::BatchJobChunkRow::list_by_job(&mut conn, job_id, tenant_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let failure_count =
            BatchJobItemFailureRow::count_by_job(&mut conn, job_id, tenant_id, None)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(BatchJobDetail {
            job: job_row.into(),
            chunks: chunk_rows.into_iter().map(Into::into).collect(),
            failure_count,
        })
    }

    async fn list_batch_jobs(
        &self,
        tenant_id: TenantId,
        job_type: Option<BatchJobTypeEnum>,
        status: Option<Vec<BatchJobStatusEnum>>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<BatchJob>> {
        let mut conn = self.get_conn().await?;

        let res = BatchJobRow::list_by_tenant(
            &mut conn,
            tenant_id,
            job_type.map(Into::into),
            status.map(|v| v.into_iter().map(Into::into).collect()),
            pagination.into(),
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(PaginatedVec {
            items: res.items.into_iter().map(Into::into).collect(),
            total_pages: res.total_pages,
            total_results: res.total_results,
        })
    }

    async fn check_duplicate_batch_job(
        &self,
        tenant_id: TenantId,
        job_type: BatchJobTypeEnum,
        file_hash: &str,
    ) -> StoreResult<Option<BatchJob>> {
        let mut conn = self.get_conn().await?;

        let row =
            BatchJobRow::find_active_by_file_hash(&mut conn, tenant_id, job_type.into(), file_hash)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(row.map(Into::into))
    }

    async fn check_completed_duplicate_batch_job(
        &self,
        tenant_id: TenantId,
        job_type: BatchJobTypeEnum,
        file_hash: &str,
    ) -> StoreResult<Option<BatchJob>> {
        let mut conn = self.get_conn().await?;

        let row = BatchJobRow::find_completed_by_file_hash(
            &mut conn,
            tenant_id,
            job_type.into(),
            file_hash,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(row.map(Into::into))
    }

    async fn cancel_batch_job(&self, job_id: BatchJobId, tenant_id: TenantId) -> StoreResult<()> {
        self.transaction(|conn| {
            async move {
                diesel_models::batch_jobs::BatchJobChunkRow::cancel_pending_for_job(
                    conn, job_id, tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                BatchJobRow::cancel(conn, job_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn retry_failed_chunks(
        &self,
        job_id: BatchJobId,
        tenant_id: TenantId,
    ) -> StoreResult<u32> {
        self.transaction(|conn| {
            async move {
                BatchJobItemFailureRow::delete_for_failed_chunks(conn, job_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let retried =
                    diesel_models::batch_jobs::BatchJobChunkRow::retry_failed_chunks_for_job(
                        conn, job_id, tenant_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                if retried > 0 {
                    BatchJobRow::reopen_for_retry(conn, job_id, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                }

                Ok(retried as u32)
            }
            .scope_boxed()
        })
        .await
    }

    async fn list_batch_job_failures(
        &self,
        job_id: BatchJobId,
        tenant_id: TenantId,
        chunk_id: Option<BatchJobChunkId>,
        limit: i64,
        offset: i64,
    ) -> StoreResult<Vec<BatchJobItemFailure>> {
        let mut conn = self.get_conn().await?;

        let rows = BatchJobItemFailureRow::list_by_job(
            &mut conn,
            job_id,
            tenant_id,
            chunk_id.map(|id| *id),
            limit,
            offset,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn count_batch_job_failures(
        &self,
        job_id: BatchJobId,
        tenant_id: TenantId,
        chunk_id: Option<BatchJobChunkId>,
    ) -> StoreResult<i64> {
        let mut conn = self.get_conn().await?;

        let count = BatchJobItemFailureRow::count_by_job(
            &mut conn,
            job_id,
            tenant_id,
            chunk_id.map(|id| *id),
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(count)
    }

    // --- Worker-facing methods ---

    async fn get_batch_job_unscoped(&self, job_id: BatchJobId) -> StoreResult<BatchJob> {
        let mut conn = self.get_conn().await?;
        let row = BatchJobRow::find_by_id_unscoped(&mut conn, job_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;
        Ok(row.into())
    }

    async fn claim_pending_job(&self) -> StoreResult<Option<BatchJob>> {
        let mut conn = self.get_conn().await?;

        let row = BatchJobRow::claim_pending_job(&mut conn)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(row.map(Into::into))
    }

    async fn complete_chunking(
        &self,
        job_id: BatchJobId,
        tenant_id: TenantId,
        chunks: Vec<(i32, i32)>,
        total_items: i32,
        max_retries: i32,
    ) -> StoreResult<()> {
        let chunk_rows: Vec<BatchJobChunkRowNew> = chunks
            .into_iter()
            .enumerate()
            .map(|(idx, (offset, count))| BatchJobChunkRowNew {
                id: BatchJobChunkId::new(),
                job_id,
                tenant_id,
                chunk_index: idx as i32,
                status: diesel_enums::BatchJobChunkStatusEnum::Pending,
                item_offset: offset,
                item_count: count,
                max_retries,
            })
            .collect();

        self.transaction(|conn| {
            async move {
                BatchJobChunkRowNew::insert_batch(conn, &chunk_rows)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                BatchJobRow::mark_as_processing(conn, job_id, total_items)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn fail_job(&self, job_id: BatchJobId, error: Option<String>) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        BatchJobRow::mark_as_failed(&mut conn, job_id, error)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(())
    }

    async fn claim_pending_chunk(&self) -> StoreResult<Option<BatchJobChunk>> {
        let mut conn = self.get_conn().await?;

        let row = diesel_models::batch_jobs::BatchJobChunkRow::claim_pending_chunk(&mut conn)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(row.map(Into::into))
    }

    async fn complete_chunk(
        &self,
        chunk_id: BatchJobChunkId,
        job_id: BatchJobId,
        tenant_id: TenantId,
        processed: i32,
        failed: i32,
        failures: Vec<BatchJobItemFailureInput>,
    ) -> StoreResult<bool> {
        self.transaction(|conn| {
            async move {
                // CAS update first: if 0 rows affected, this worker was evicted (zombie)
                let updated = diesel_models::batch_jobs::BatchJobChunkRow::mark_as_completed(
                    conn, chunk_id, processed, failed,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                if updated == 0 {
                    return Ok(false);
                }

                // Only insert failures if we still own the chunk
                let failure_rows: Vec<BatchJobItemFailureRowNew> = failures
                    .into_iter()
                    .map(|f| BatchJobItemFailureRowNew {
                        chunk_id,
                        job_id,
                        tenant_id,
                        item_index: f.item_index,
                        item_identifier: f.item_identifier,
                        reason: f.reason,
                    })
                    .collect();

                if !failure_rows.is_empty() {
                    BatchJobItemFailureRowNew::insert_batch(conn, &failure_rows)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                }

                Ok(true)
            }
            .scope_boxed()
        })
        .await
    }

    async fn fail_chunk(
        &self,
        chunk_id: BatchJobChunkId,
        error_message: &str,
    ) -> StoreResult<bool> {
        let mut conn = self.get_conn().await?;

        let updated = diesel_models::batch_jobs::BatchJobChunkRow::mark_as_failed(
            &mut conn,
            chunk_id,
            error_message,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(updated > 0)
    }

    async fn schedule_chunk_retry(
        &self,
        chunk_id: BatchJobChunkId,
        retry_after: chrono::NaiveDateTime,
        error_message: &str,
    ) -> StoreResult<bool> {
        let mut conn = self.get_conn().await?;

        let updated = diesel_models::batch_jobs::BatchJobChunkRow::schedule_retry(
            &mut conn,
            chunk_id,
            retry_after,
            error_message,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(updated > 0)
    }

    async fn append_chunk_event(
        &self,
        chunk_id: BatchJobChunkId,
        event_type: &str,
        attempt: i32,
        message: Option<&str>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        diesel_models::batch_jobs::BatchJobChunkRow::append_event(
            &mut conn, chunk_id, event_type, attempt, message,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(())
    }

    async fn record_chunk_item_failures(
        &self,
        chunk_id: BatchJobChunkId,
        job_id: BatchJobId,
        tenant_id: TenantId,
        failures: Vec<BatchJobItemFailureInput>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let failure_rows: Vec<BatchJobItemFailureRowNew> = failures
            .into_iter()
            .map(|f| BatchJobItemFailureRowNew {
                chunk_id,
                job_id,
                tenant_id,
                item_index: f.item_index,
                item_identifier: f.item_identifier,
                reason: f.reason,
            })
            .collect();

        if !failure_rows.is_empty() {
            BatchJobItemFailureRowNew::insert_batch(&mut conn, &failure_rows)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;
        }

        Ok(())
    }

    async fn try_finalize_job(
        &self,
        job_id: BatchJobId,
    ) -> StoreResult<Option<BatchJobStatusEnum>> {
        self.transaction(|conn| {
            async move {
                let summary = diesel_models::batch_jobs::BatchJobChunkRow::get_job_chunk_summary(
                    conn, job_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                if summary.active_chunks > 0 {
                    return Ok(None);
                }

                let final_status = if summary.failed_chunks == summary.total_chunks {
                    BatchJobStatusEnum::Failed
                } else if summary.failed_chunks > 0 || summary.total_failed > 0 {
                    BatchJobStatusEnum::CompletedWithErrors
                } else {
                    BatchJobStatusEnum::Completed
                };

                let updated = BatchJobRow::finalize(conn, job_id, final_status.clone().into())
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                if updated > 0 {
                    Ok(Some(final_status))
                } else {
                    Ok(None)
                }
            }
            .scope_boxed()
        })
        .await
    }

    async fn set_error_output_key(&self, job_id: BatchJobId, key: String) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        BatchJobRow::set_error_output_key(&mut conn, job_id, key)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;
        Ok(())
    }

    async fn list_failed_item_indices(
        &self,
        job_id: BatchJobId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<(i32, String)>> {
        let mut conn = self.get_conn().await?;
        let mut all = Vec::new();
        let page_size: i64 = 10_000;
        let mut offset: i64 = 0;

        loop {
            let rows = BatchJobItemFailureRow::list_by_job(
                &mut conn, job_id, tenant_id, None, page_size, offset,
            )
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

            let count = rows.len() as i64;
            all.extend(rows.into_iter().map(|r| (r.item_index, r.reason)));

            if count < page_size {
                break;
            }
            offset += count;
        }

        Ok(all)
    }

    async fn reset_stuck_items(&self) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        BatchJobRow::reset_stuck_chunking_jobs(&mut conn)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        diesel_models::batch_jobs::BatchJobChunkRow::reset_stuck_chunks(&mut conn)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(())
    }

    async fn find_stalled_processing_jobs(&self) -> StoreResult<Vec<BatchJobId>> {
        let mut conn = self.get_conn().await?;

        let ids = BatchJobRow::find_stalled_processing_jobs(&mut conn)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(ids)
    }

    async fn fail_timed_out_jobs(&self, max_age: chrono::Duration) -> StoreResult<Vec<BatchJobId>> {
        let mut conn = self.get_conn().await?;

        let ids = BatchJobRow::fail_timed_out_jobs(&mut conn, max_age)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(ids)
    }

    async fn record_batch_job_entities(&self, entities: Vec<BatchJobEntityNew>) -> StoreResult<()> {
        if entities.is_empty() {
            return Ok(());
        }

        let mut conn = self.get_conn().await?;

        let rows: Vec<BatchJobEntityRowNew> = entities
            .into_iter()
            .map(|e| BatchJobEntityRowNew {
                batch_job_id: e.batch_job_id,
                tenant_id: e.tenant_id,
                entity_type: e.entity_type,
                entity_id: e.entity_id,
            })
            .collect();

        BatchJobEntityRowNew::insert_batch(&mut conn, &rows)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(())
    }
}
