use async_trait::async_trait;
use bytes::Bytes;
use meteroid_store::domain::batch_jobs::{BatchJob, BatchJobChunk};

/// A chunk definition produced during the chunking phase.
#[derive(Debug)]
pub struct ChunkDefinition {
    pub item_offset: i32,
    pub item_count: i32,
}

/// Result of processing a single item within a chunk.
#[derive(Debug)]
pub struct ItemFailure {
    pub item_index: i32,
    pub item_identifier: Option<String>,
    pub reason: String,
}

/// An entity created or updated by a batch job processor.
#[derive(Debug)]
pub struct CreatedEntity {
    pub entity_type: &'static str,
    pub entity_id: uuid::Uuid,
}

/// Result of processing a chunk.
#[derive(Debug)]
pub struct ChunkResult {
    pub processed: i32,
    pub failures: Vec<ItemFailure>,
    pub created_entities: Vec<CreatedEntity>,
}

/// Trait for implementing batch job processors.
///
/// Each job type (CSV import, plan migration, etc.) implements this trait.
/// The engine handles state transitions, parallelism, retries, and failure tracking.
#[async_trait]
pub trait BatchJobProcessor: Send + Sync {
    /// Parse/validate the input and split it into chunks.
    ///
    /// Called once per job during the CHUNKING phase. The processor should:
    /// 1. Read the input from S3 (or from input_params for non-file jobs)
    /// 2. Validate the input format
    /// 3. Return chunk definitions (offset + count for each chunk)
    ///
    /// If this fails, the job transitions to FAILED.
    async fn prepare_chunks(
        &self,
        job: &BatchJob,
        input_data: Option<Bytes>,
    ) -> Result<Vec<ChunkDefinition>, String>;

    /// Process a single chunk of items.
    ///
    /// Called once per chunk during the PROCESSING phase. The processor should:
    /// 1. Read the relevant slice of input (using chunk offset/count)
    /// 2. Process each item
    /// 3. Return results with per-item failures
    ///
    /// The input_data is the full file content, cached per-job by the worker.
    async fn process_chunk(
        &self,
        job: &BatchJob,
        chunk: &BatchJobChunk,
        input_data: Option<Bytes>,
    ) -> Result<ChunkResult, String>;

    /// Reduce/finalize hook, called once after the job reaches a terminal status and all
    /// per-chunk work is done — the map-reduce "reduce" step. The default is a no-op, so
    /// stateless processors (the CSV imports) are unaffected. Aggregating jobs (the tax
    /// report export) override this to combine the per-chunk partial artifacts into the
    /// job's single output and record it via `set_output_key`.
    ///
    /// `job` is re-read after finalization, so `job.status` reflects the terminal state
    /// and the processor can decide whether to assemble output (e.g. only on a clean
    /// `Completed`).
    async fn finalize_job(&self, _job: &BatchJob) -> Result<(), String> {
        Ok(())
    }

    /// Max retries for chunks of this job type.
    fn max_retries(&self) -> i32 {
        3
    }

    /// Chunk size for this job type.
    fn chunk_size(&self) -> i32 {
        500
    }
}
