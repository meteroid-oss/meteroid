use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use common_domain::ids::BatchJobId;
use meteroid_store::Store;
use meteroid_store::domain::batch_jobs::{BatchJob, BatchJobEntityNew, BatchJobItemFailureInput};
use meteroid_store::domain::enums::{
    BatchJobChunkStatusEnum, BatchJobStatusEnum, BatchJobTypeEnum,
};
use meteroid_store::repositories::batch_jobs::BatchJobsInterface;
use tokio::sync::Mutex;

use crate::services::csv_ingest::normalize_csv_encoding;
use crate::services::storage::{ObjectStoreService, Prefix, S3Storage};
use crate::workers::batch_jobs::engine::BatchJobProcessor;
use crate::workers::pgmq::sleep_with_jitter;

const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const RETRY_BACKOFFS_SECS: &[u64] = &[10, 30, 90];
fn max_job_age() -> chrono::Duration {
    chrono::Duration::hours(4)
}

fn retry_backoff(retry_count: i32) -> Duration {
    let idx = (retry_count as usize).min(RETRY_BACKOFFS_SECS.len() - 1);
    Duration::from_secs(RETRY_BACKOFFS_SECS[idx])
}

pub struct BatchJobWorker {
    store: Arc<Store>,
    object_store: Arc<S3Storage>,
    processors: HashMap<BatchJobTypeEnum, Arc<dyn BatchJobProcessor>>,
    input_cache: Mutex<Option<(BatchJobId, Option<Bytes>)>>,
}

impl BatchJobWorker {
    pub fn new(store: Arc<Store>, object_store: Arc<S3Storage>) -> Self {
        Self {
            store,
            object_store,
            processors: HashMap::new(),
            input_cache: Mutex::new(None),
        }
    }

    pub fn register_processor(
        &mut self,
        job_type: BatchJobTypeEnum,
        processor: Arc<dyn BatchJobProcessor>,
    ) {
        self.processors.insert(job_type, processor);
    }

    fn get_processor(&self, job_type: &BatchJobTypeEnum) -> Option<&Arc<dyn BatchJobProcessor>> {
        self.processors.get(job_type)
    }

    async fn fetch_input_data(&self, job: &BatchJob) -> Result<Option<Bytes>, String> {
        match &job.input_source_key {
            Some(key) => {
                let prefix = Prefix::BatchJobInput {
                    tenant_id: job.tenant_id,
                };
                let doc_id = key
                    .parse()
                    .map_err(|e| format!("Invalid stored document id: {e}"))?;
                let data = self
                    .object_store
                    .retrieve(doc_id, prefix)
                    .await
                    .map_err(|e| format!("Failed to fetch input from S3: {e}"))?;
                Ok(Some(data))
            }
            None => Ok(None),
        }
    }

    /// Fetch input data with per-job caching. Avoids re-downloading the same file
    /// from S3 for every chunk of the same job.
    async fn fetch_input_data_cached(&self, job: &BatchJob) -> Result<Option<Bytes>, String> {
        let cache = self.input_cache.lock().await;
        if let Some((cached_job_id, ref data)) = *cache
            && cached_job_id == job.id
        {
            return Ok(data.clone());
        }
        // Cache miss — download and store
        drop(cache);
        let data = self.fetch_input_data(job).await?;
        let mut cache = self.input_cache.lock().await;
        *cache = Some((job.id, data.clone()));
        Ok(data)
    }

    pub async fn run(self: Arc<Self>) {
        let mut last_cleanup = Instant::now();

        loop {
            let mut did_work = false;

            // 1. Try to claim a pending job for chunking
            match self.try_chunk_job().await {
                Ok(true) => {
                    did_work = true;
                }
                Ok(false) => {}
                Err(e) => {
                    log::error!("Batch job chunking error: {e}");
                    sleep_with_jitter(tokio::time::Duration::from_secs(1)).await;
                    continue;
                }
            }

            // 2. Try to claim a pending chunk for processing
            match self.try_process_chunk().await {
                Ok(true) => {
                    did_work = true;
                }
                Ok(false) => {}
                Err(e) => {
                    log::error!("Batch job chunk processing error: {e}");
                    sleep_with_jitter(tokio::time::Duration::from_secs(1)).await;
                    continue;
                }
            }

            // 3. Cleanup stuck items and finalize stalled jobs periodically
            if last_cleanup.elapsed() > CLEANUP_INTERVAL {
                if let Err(e) = self.store.reset_stuck_items().await {
                    log::warn!("Batch job stuck items cleanup failed: {e:?}");
                }

                // Abort jobs that have exceeded the maximum allowed age.
                // Skips their PENDING chunks so the stalled-job sweep can finalize them.
                match self.store.fail_timed_out_jobs(max_job_age()).await {
                    Ok(timed_out) => {
                        for job_id in &timed_out {
                            log::warn!(
                                "Job {} timed out after {:?}, finalizing",
                                job_id,
                                max_job_age()
                            );
                            self.try_finalize(*job_id).await;
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to check for timed-out jobs: {e:?}");
                    }
                }

                // Finalize PROCESSING jobs whose chunks are all in terminal state.
                // Covers: crash recovery marking chunks FAILED, transient try_finalize failures.
                match self.store.find_stalled_processing_jobs().await {
                    Ok(stalled) => {
                        for job_id in stalled {
                            log::info!("Finalizing stalled job {}", job_id);
                            self.try_finalize(job_id).await;
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to find stalled processing jobs: {e:?}");
                    }
                }

                last_cleanup = Instant::now();
            }

            if !did_work {
                sleep_with_jitter(tokio::time::Duration::from_millis(
                    POLL_INTERVAL.as_millis() as u64,
                ))
                .await;
            }
        }
    }

    async fn try_chunk_job(&self) -> Result<bool, String> {
        let job = match self
            .store
            .claim_pending_job()
            .await
            .map_err(|e| format!("claim_pending_job: {e:?}"))?
        {
            Some(j) => j,
            None => return Ok(false),
        };

        log::info!(
            "Claimed batch job {} ({:?}) for chunking",
            job.id,
            job.job_type
        );

        let processor = match self.get_processor(&job.job_type) {
            Some(p) => p.clone(),
            None => {
                let msg = format!("No processor registered for job type {:?}", job.job_type);
                log::error!("{}", msg);
                let _ = self.store.fail_job(job.id, Some(msg)).await;
                return Ok(true);
            }
        };

        let input_data = match self.fetch_input_data(&job).await {
            Ok(data) => data,
            Err(e) => {
                let msg = format!("Failed to fetch input: {e}");
                log::error!("Failed to fetch input for job {}: {}", job.id, e);
                let _ = self.store.fail_job(job.id, Some(msg)).await;
                return Ok(true);
            }
        };

        let chunks = match processor.prepare_chunks(&job, input_data).await {
            Ok(c) => c,
            Err(e) => {
                let msg = e.to_string();
                log::error!("Failed to prepare chunks for job {}: {}", job.id, e);
                let _ = self.store.fail_job(job.id, Some(msg)).await;
                return Ok(true);
            }
        };

        let total_items: i32 = chunks.iter().map(|c| c.item_count).sum();
        let chunk_defs: Vec<(i32, i32)> = chunks
            .into_iter()
            .map(|c| (c.item_offset, c.item_count))
            .collect();

        self.store
            .complete_chunking(
                job.id,
                job.tenant_id,
                chunk_defs,
                total_items,
                processor.max_retries(),
            )
            .await
            .map_err(|e| format!("complete_chunking: {e:?}"))?;

        log::info!("Job {} chunked: {} items", job.id, total_items);

        Ok(true)
    }

    async fn try_process_chunk(&self) -> Result<bool, String> {
        let chunk = match self
            .store
            .claim_pending_chunk()
            .await
            .map_err(|e| format!("claim_pending_chunk: {e:?}"))?
        {
            Some(c) => c,
            None => return Ok(false),
        };

        log::debug!(
            "Claimed chunk {} (job {}, index {})",
            chunk.id,
            chunk.job_id,
            chunk.chunk_index
        );

        let job_detail = self
            .store
            .get_batch_job(chunk.job_id, chunk.tenant_id)
            .await
            .map_err(|e| format!("get_batch_job: {e:?}"))?;

        let processor = match self.get_processor(&job_detail.job.job_type) {
            Some(p) => p.clone(),
            None => {
                log::error!(
                    "No processor for job type {:?} (chunk {})",
                    job_detail.job.job_type,
                    chunk.id
                );
                let owns = match self
                    .store
                    .fail_chunk(chunk.id, "No processor registered")
                    .await
                {
                    Ok(true) => true,
                    Ok(false) => {
                        log::warn!(
                            "Chunk {} was reclaimed by another worker (zombie eviction), skipping",
                            chunk.id
                        );
                        false
                    }
                    Err(_) => false,
                };
                if owns {
                    self.try_finalize(chunk.job_id).await;
                }
                return Ok(true);
            }
        };

        let input_data = match self.fetch_input_data_cached(&job_detail.job).await {
            Ok(data) => data,
            Err(e) => {
                log::error!(
                    "Failed to fetch input for chunk {} (job {}): {}",
                    chunk.id,
                    chunk.job_id,
                    e
                );
                let owns = match self.store.fail_chunk(chunk.id, &e).await {
                    Ok(true) => true,
                    Ok(false) => {
                        log::warn!(
                            "Chunk {} was reclaimed by another worker (zombie eviction), skipping",
                            chunk.id
                        );
                        false
                    }
                    Err(_) => false,
                };
                if owns {
                    self.try_finalize(chunk.job_id).await;
                }
                return Ok(true);
            }
        };

        let _ = self
            .store
            .append_chunk_event(chunk.id, "STARTED", chunk.retry_count + 1, None)
            .await;

        let owns_chunk = match processor
            .process_chunk(&job_detail.job, &chunk, input_data)
            .await
        {
            Ok(result) => {
                let failures: Vec<BatchJobItemFailureInput> = result
                    .failures
                    .into_iter()
                    .map(|f| BatchJobItemFailureInput {
                        item_index: f.item_index,
                        item_identifier: f.item_identifier,
                        reason: f.reason,
                    })
                    .collect();

                let failed = failures.len() as i32;

                let owns = match self
                    .store
                    .complete_chunk(
                        chunk.id,
                        chunk.job_id,
                        chunk.tenant_id,
                        result.processed,
                        failed,
                        failures,
                    )
                    .await
                {
                    Ok(true) => true,
                    Ok(false) => {
                        log::warn!(
                            "Chunk {} was reclaimed by another worker (zombie eviction), skipping",
                            chunk.id
                        );
                        false
                    }
                    Err(e) => {
                        log::error!("Failed to complete chunk {}: {e:?}", chunk.id);
                        false
                    }
                };

                if owns && !result.created_entities.is_empty() {
                    let entities = result
                        .created_entities
                        .into_iter()
                        .map(|e| BatchJobEntityNew {
                            batch_job_id: chunk.job_id,
                            tenant_id: chunk.tenant_id,
                            entity_type: e.entity_type.to_string(),
                            entity_id: e.entity_id,
                        })
                        .collect();

                    if let Err(e) = self.store.record_batch_job_entities(entities).await {
                        log::error!(
                            "Failed to record entities for chunk {} (job {}): {e:?}",
                            chunk.id,
                            chunk.job_id
                        );
                    }
                }

                log::debug!(
                    "Chunk {} completed: {} processed, {} failed",
                    chunk.id,
                    result.processed,
                    failed
                );

                owns
            }
            Err(e) => {
                log::error!(
                    "Chunk {} processing failed (attempt {}): {}",
                    chunk.id,
                    chunk.retry_count + 1,
                    e
                );

                if chunk.retry_count < chunk.max_retries {
                    let backoff = retry_backoff(chunk.retry_count);
                    let retry_after = chrono::Utc::now().naive_utc()
                        + chrono::Duration::from_std(backoff).unwrap();

                    match self
                        .store
                        .schedule_chunk_retry(chunk.id, retry_after, &e)
                        .await
                    {
                        Ok(true) => {
                            log::info!(
                                "Chunk {} retry {}/{} scheduled after {}s",
                                chunk.id,
                                chunk.retry_count + 1,
                                chunk.max_retries,
                                backoff.as_secs()
                            );
                        }
                        Ok(false) => {
                            log::warn!("Chunk {} reclaimed, skip retry", chunk.id);
                        }
                        Err(err) => {
                            log::error!("schedule_chunk_retry failed for {}: {err:?}", chunk.id);
                        }
                    }
                    // Chunk goes back to PENDING with retry_after — do not finalize
                    false
                } else {
                    // Retries exhausted — mark FAILED. The error CSV generator will
                    // include all rows from FAILED chunks using the chunk-level error
                    // from events, without needing per-row failure records.
                    match self.store.fail_chunk(chunk.id, &e).await {
                        Ok(true) => true,
                        Ok(false) => {
                            log::warn!("Chunk {} was reclaimed (zombie), skipping", chunk.id);
                            false
                        }
                        Err(err) => {
                            log::error!(
                                "fail_chunk failed for exhausted chunk {}: {err:?}",
                                chunk.id
                            );
                            false
                        }
                    }
                }
            }
        };

        // Only finalize if we still owned the chunk — zombie workers must not interfere
        if owns_chunk {
            self.try_finalize(chunk.job_id).await;
        }

        // Throttle between chunks to avoid overwhelming the metering service
        // when multiple workers process chunks concurrently.
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        Ok(true)
    }

    async fn try_finalize(&self, job_id: BatchJobId) {
        match self.store.try_finalize_job(job_id).await {
            Ok(Some(status)) => {
                log::info!("Job {} finalized with status {:?}", job_id, status);

                if matches!(
                    status,
                    BatchJobStatusEnum::CompletedWithErrors | BatchJobStatusEnum::Failed
                ) && let Err(e) = self.generate_error_csv(job_id).await
                {
                    log::error!("Failed to generate error CSV for job {}: {}", job_id, e);
                }
            }
            Ok(None) => {}
            Err(e) => {
                log::error!("Failed to finalize job {}: {e:?}", job_id);
            }
        }
    }

    /// Build a CSV containing only the failed rows (with original data + _error column),
    /// upload it to S3, and save the document key on the job.
    ///
    /// Safe with multiple workers: `try_finalize_job` uses CAS (only one succeeds),
    /// so only the finalizing worker calls this. `set_error_output_key` also uses CAS
    /// (error_output_key IS NULL) as a secondary guard.
    async fn generate_error_csv(&self, job_id: BatchJobId) -> Result<(), String> {
        let job = self
            .store
            .get_batch_job_unscoped(job_id)
            .await
            .map_err(|e| format!("get_batch_job_unscoped: {e:?}"))?;

        let tenant_id = job.tenant_id;
        let input_key = job
            .input_source_key
            .ok_or("Job has no input_source_key — cannot generate error CSV")?;

        let doc_id = input_key
            .parse()
            .map_err(|e| format!("Invalid stored document id: {e}"))?;
        let original_data = self
            .object_store
            .retrieve(doc_id, Prefix::BatchJobInput { tenant_id })
            .await
            .map_err(|e| format!("Failed to retrieve original CSV: {e}"))?;

        // Collect per-row failures from batch_job_item_failure records
        let failed_items = self
            .store
            .list_failed_item_indices(job_id, tenant_id)
            .await
            .map_err(|e| format!("list_failed_item_indices: {e:?}"))?;

        let mut error_map: HashMap<i32, String> = HashMap::new();
        for (idx, reason) in failed_items {
            error_map
                .entry(idx)
                .and_modify(|existing| {
                    existing.push_str("; ");
                    existing.push_str(&reason);
                })
                .or_insert(reason);
        }

        // For FAILED chunks (exhausted retries), include ALL their rows with the
        // chunk-level error from events. No per-row failure records exist for these.
        let detail = self
            .store
            .get_batch_job(job_id, tenant_id)
            .await
            .map_err(|e| format!("get_batch_job: {e:?}"))?;

        for chunk in &detail.chunks {
            if chunk.status != BatchJobChunkStatusEnum::Failed {
                continue;
            }
            let error_msg = chunk
                .events
                .iter()
                .rev()
                .find(|e| e.event == "EXHAUSTED" || e.event == "ERRORED")
                .and_then(|e| e.message.clone())
                .unwrap_or_else(|| "Chunk failed".to_string());

            for i in 0..chunk.item_count {
                let row_idx = chunk.item_offset + i;
                error_map
                    .entry(row_idx)
                    .or_insert_with(|| error_msg.clone());
            }
        }

        if error_map.is_empty() {
            return Ok(());
        }

        let delimiter = job
            .input_params
            .as_ref()
            .and_then(|p| p.get("delimiter"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.chars().next())
            .unwrap_or(',') as u8;

        let normalized = normalize_csv_encoding(&original_data);
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .delimiter(delimiter)
            .from_reader(normalized.as_ref());

        let headers = reader
            .headers()
            .map_err(|e| format!("Failed to read CSV headers: {e}"))?
            .clone();

        let mut wtr = csv::WriterBuilder::new()
            .delimiter(delimiter)
            .from_writer(Vec::new());

        let mut header_record = headers.clone();
        header_record.push_field("_error");
        wtr.write_record(&header_record)
            .map_err(|e| format!("write header: {e}"))?;

        for (row_idx, result) in reader.records().enumerate() {
            let record = match result {
                Ok(r) => r,
                Err(_) => continue,
            };

            if let Some(error_reason) = error_map.get(&(row_idx as i32)) {
                let mut fields: Vec<&str> = record.iter().collect();
                fields.push(error_reason);
                wtr.write_record(&fields)
                    .map_err(|e| format!("write row: {e}"))?;
            }
        }

        let csv_bytes = wtr.into_inner().map_err(|e| format!("flush csv: {e}"))?;

        let prefix = Prefix::BatchJobErrorOutput { tenant_id };
        let doc_id = self
            .object_store
            .store(Bytes::from(csv_bytes), prefix)
            .await
            .map_err(|e| format!("Failed to store error CSV: {e}"))?;

        self.store
            .set_error_output_key(job_id, doc_id.to_string())
            .await
            .map_err(|e| format!("set_error_output_key: {e:?}"))?;

        log::info!(
            "Generated error CSV for job {} ({} failed rows)",
            job_id,
            error_map.len()
        );

        Ok(())
    }
}
