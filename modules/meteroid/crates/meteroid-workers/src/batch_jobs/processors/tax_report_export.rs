//! Tax-report transactions CSV export processor — the scalable replacement for building
//! the (potentially huge) per-invoice transaction trail in one synchronous response.
//!
//! Only the transaction trail can grow without bound (100k+ rows); the three summary
//! reports are small aggregates served inline by the synchronous endpoint. So this job
//! exports just the transactions, as a chunked stream:
//! - **map** (`process_chunk`): each chunk owns a contiguous date sub-range and streams
//!   that range's invoices a page at a time, writing each invoice's transaction row to a
//!   CSV *segment* in object storage keyed by `(job_id, chunk_index)`. Memory stays
//!   `O(page_size)` — invoices are never all held at once.
//! - **reduce** (`finalize_job`): once every chunk has completed, the segments are
//!   concatenated in chunk (= date) order under one header into the final CSV.
//!
//! Contiguous date sub-ranges are already `(invoice_date, id)`-ordered, so the segments
//! concatenate into globally sorted transactions with no global sort.

use std::sync::Arc;

use crate::batch_jobs::engine::{BatchJobProcessor, ChunkDefinition, ChunkResult};
use async_trait::async_trait;
use bytes::Bytes;
use chrono::{Duration, NaiveDate};
use common_domain::ids::{BatchJobId, InvoiceId};
use meteroid_app::services::storage::{self, ObjectKind, ObjectStoreService};
use meteroid_store::Store;
use meteroid_store::domain::batch_jobs::{BatchJob, BatchJobChunk};
use meteroid_store::domain::enums::{BatchJobChunkStatusEnum, BatchJobStatusEnum};
use meteroid_store::domain::tax_report::{TaxTransactionRow, transaction_row};
use meteroid_store::repositories::batch_jobs::BatchJobsInterface;
use meteroid_store::repositories::invoices::InvoiceInterface;

/// Default width of a date sub-range chunk when the caller does not specify one.
const DEFAULT_BUCKET_DAYS: i64 = 7;
/// Upper bound on the number of chunks, so a very long period does not create thousands
/// of tiny chunks. The bucket widens to respect this.
const MAX_CHUNKS: i64 = 90;
/// Invoices fetched per page while streaming a chunk's date range.
const PAGE_SIZE: i64 = 1000;

pub use meteroid_app::workers::batch_jobs::TaxReportExportParams;

pub struct TaxReportExportProcessor {
    store: Arc<Store>,
    object_store: Arc<dyn ObjectStoreService>,
}

impl TaxReportExportProcessor {
    pub fn new(store: Arc<Store>, object_store: Arc<dyn ObjectStoreService>) -> Self {
        Self {
            store,
            object_store,
        }
    }

    fn params(job: &BatchJob) -> Result<TaxReportExportParams, String> {
        let value = job
            .input_params
            .clone()
            .ok_or("tax report export job is missing input_params")?;
        serde_json::from_value(value).map_err(|e| format!("invalid tax report params: {e}"))
    }
}

const TRANSACTION_HEADER: [&str; 11] = [
    "invoice_number",
    "invoice_date",
    "customer_name",
    "vat_number",
    "country",
    "currency",
    "net_amount",
    "tax_amount",
    "gross_amount",
    "status",
    "exemption",
];

fn transaction_fields(r: &TaxTransactionRow) -> Vec<String> {
    vec![
        r.invoice_number.clone(),
        r.invoice_date.to_string(),
        r.customer_name.clone(),
        r.vat_number.clone().unwrap_or_default(),
        r.country.clone().unwrap_or_default(),
        r.currency.clone(),
        r.net_amount.to_string(),
        r.tax_amount.to_string(),
        r.gross_amount.to_string(),
        r.status.clone(),
        r.exemption.clone().unwrap_or_default(),
    ]
}

/// The single CSV header row the concatenated segments sit under. Amounts in each
/// segment are minor units in the row's own currency (matching the domain / the
/// synchronous report), so the numbers reconcile exactly.
fn transaction_header_csv() -> Result<Vec<u8>, String> {
    let mut wtr = csv::WriterBuilder::new().from_writer(Vec::new());
    wtr.write_record(TRANSACTION_HEADER)
        .map_err(|e| format!("write header: {e}"))?;
    wtr.into_inner().map_err(|e| format!("flush header: {e}"))
}

fn segment_key(job_id: BatchJobId, chunk_index: i32) -> String {
    format!("{job_id}-chunk-{chunk_index:06}-segment.csv")
}

#[async_trait]
impl BatchJobProcessor for TaxReportExportProcessor {
    async fn prepare_chunks(
        &self,
        job: &BatchJob,
        _input_data: Option<Bytes>,
    ) -> Result<Vec<ChunkDefinition>, String> {
        let params = Self::params(job)?;
        if params.period_end < params.period_start {
            return Err("period_end precedes period_start".to_string());
        }

        // Inclusive day span of the reporting period.
        let total_days = (params.period_end - params.period_start).num_days() + 1;

        let requested = params.bucket_days.unwrap_or(DEFAULT_BUCKET_DAYS).max(1);
        // Widen the bucket if needed to keep the chunk count bounded (ceil division;
        // signed `div_ceil` is still unstable, so do it by hand — both are positive).
        let min_bucket = (total_days + MAX_CHUNKS - 1) / MAX_CHUNKS;
        let bucket = requested.max(min_bucket).max(1);

        // One chunk per contiguous date sub-range; item_offset/item_count encode the
        // day-offset and day-span, so process_chunk can recover its range with no extra
        // per-chunk state.
        let mut chunks = Vec::new();
        let mut offset: i64 = 0;
        while offset < total_days {
            let count = bucket.min(total_days - offset);
            chunks.push(ChunkDefinition {
                item_offset: offset as i32,
                item_count: count as i32,
            });
            offset += bucket;
        }

        Ok(chunks)
    }

    async fn process_chunk(
        &self,
        job: &BatchJob,
        chunk: &BatchJobChunk,
        _input_data: Option<Bytes>,
    ) -> Result<ChunkResult, String> {
        let params = Self::params(job)?;
        let tenant_id = job.tenant_id;

        let sub_from = params
            .period_start
            .checked_add_signed(Duration::days(chunk.item_offset as i64))
            .ok_or("chunk start date overflow")?;
        let sub_to = params
            .period_start
            .checked_add_signed(Duration::days(
                (chunk.item_offset + chunk.item_count - 1) as i64,
            ))
            .ok_or("chunk end date overflow")?;

        let mut wtr = csv::WriterBuilder::new().from_writer(Vec::new());
        let mut invoice_count: i64 = 0;
        let mut after: Option<(NaiveDate, InvoiceId)> = None;

        loop {
            let page = self
                .store
                .list_invoices_for_tax_report_page(
                    tenant_id,
                    sub_from,
                    sub_to,
                    params.invoicing_entity_id,
                    after,
                    PAGE_SIZE,
                )
                .await
                .map_err(|e| format!("list invoices page: {e:?}"))?;

            if page.is_empty() {
                break;
            }

            for inv in &page {
                wtr.write_record(transaction_fields(&transaction_row(inv)))
                    .map_err(|e| format!("write transaction row: {e}"))?;
            }
            invoice_count += page.len() as i64;

            if (page.len() as i64) < PAGE_SIZE {
                break;
            }
            // Advance the keyset cursor past the last row of this page.
            if let Some(last) = page.last() {
                after = Some((last.invoice_date, last.id));
            }
        }

        let segment = wtr
            .into_inner()
            .map_err(|e| format!("flush segment: {e}"))?;
        self.object_store
            .put_scratch(
                ObjectKind::TaxReport,
                tenant_id,
                &segment_key(job.id, chunk.chunk_index),
                Bytes::from(segment),
            )
            .await
            .map_err(|e| format!("store transaction segment: {e:?}"))?;

        log::debug!(
            "Tax report job {} chunk {} ({}..={}): {} invoices",
            job.id,
            chunk.chunk_index,
            sub_from,
            sub_to,
            invoice_count
        );

        // Progress is measured in the same unit the chunks were sized in (days), so the
        // job's processed_items reconciles with total_items; the invoice count is logged.
        Ok(ChunkResult {
            processed: chunk.item_count,
            failures: vec![],
            created_entities: vec![],
        })
    }

    async fn finalize_job(&self, job: &BatchJob) -> Result<(), String> {
        // A partial export (some chunks failed) would drop rows, so only assemble output
        // on a clean completion; otherwise leave output_key null.
        if job.status != BatchJobStatusEnum::Completed {
            log::info!(
                "Tax report job {} finished as {:?}; skipping output assembly",
                job.id,
                job.status
            );
            return Ok(());
        }

        let tenant_id = job.tenant_id;

        let detail = self
            .store
            .get_batch_job(job.id, tenant_id)
            .await
            .map_err(|e| format!("get_batch_job: {e:?}"))?;

        let mut completed: Vec<&BatchJobChunk> = detail
            .chunks
            .iter()
            .filter(|c| c.status == BatchJobChunkStatusEnum::Completed)
            .collect();
        completed.sort_by_key(|c| c.chunk_index);

        // Reduce: one header, then every chunk's transaction segment in chunk (= date)
        // order — already globally sorted, so a plain concatenation.
        let mut output = transaction_header_csv()?;
        for chunk in &completed {
            let segment = self
                .object_store
                .read_scratch(
                    ObjectKind::TaxReport,
                    tenant_id,
                    &segment_key(job.id, chunk.chunk_index),
                )
                .await
                .map_err(|e| format!("retrieve segment (chunk {}): {e:?}", chunk.chunk_index))?;
            output.extend_from_slice(&segment);
        }
        let output_len = output.len();

        let stored = storage::put(
            self.object_store.as_ref(),
            self.store.as_ref(),
            ObjectKind::TaxReport,
            tenant_id,
            Bytes::from(output),
        )
        .await
        .map_err(|e| format!("store transactions CSV: {e:?}"))?;

        self.store
            .set_output_key(job.id, stored.id.to_string())
            .await
            .map_err(|e| format!("set_output_key: {e:?}"))?;

        // Best-effort cleanup of the per-chunk temp segments.
        for chunk in &completed {
            let _ = self
                .object_store
                .delete_scratch(
                    ObjectKind::TaxReport,
                    tenant_id,
                    &segment_key(job.id, chunk.chunk_index),
                )
                .await;
        }

        log::info!(
            "Assembled tax report transactions CSV for job {} from {} chunks ({} bytes)",
            job.id,
            completed.len(),
            output_len
        );

        Ok(())
    }

    fn max_retries(&self) -> i32 {
        3
    }
}
