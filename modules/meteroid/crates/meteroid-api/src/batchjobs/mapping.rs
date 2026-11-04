pub use meteroid_app::batch_job_files::{
    generate_error_csv_token, generate_input_file_token, generate_output_token,
};
use meteroid_app::conversions::ProtoConv;
use meteroid_grpc::meteroid::api::batchjobs::v1 as proto;
use meteroid_store::domain;
use meteroid_store::domain::enums::{
    BatchJobChunkStatusEnum, BatchJobStatusEnum, BatchJobTypeEnum,
};
use secrecy::SecretString;

pub struct BatchJobTypeWrapper(pub proto::BatchJobType);
pub struct BatchJobStatusWrapper(pub proto::BatchJobStatus);
pub struct BatchJobChunkStatusWrapper(pub proto::BatchJobChunkStatus);

impl From<BatchJobTypeWrapper> for BatchJobTypeEnum {
    fn from(val: BatchJobTypeWrapper) -> Self {
        match val.0 {
            proto::BatchJobType::EventCsvImport => BatchJobTypeEnum::EventCsvImport,
            proto::BatchJobType::CustomerCsvImport => BatchJobTypeEnum::CustomerCsvImport,
            proto::BatchJobType::SubscriptionCsvImport => BatchJobTypeEnum::SubscriptionCsvImport,
            proto::BatchJobType::SubscriptionPlanMigration => {
                BatchJobTypeEnum::SubscriptionPlanMigration
            }
            proto::BatchJobType::TaxReportExport => BatchJobTypeEnum::TaxReportExport,
        }
    }
}

impl From<BatchJobTypeEnum> for BatchJobTypeWrapper {
    fn from(e: BatchJobTypeEnum) -> Self {
        Self(match e {
            BatchJobTypeEnum::EventCsvImport => proto::BatchJobType::EventCsvImport,
            BatchJobTypeEnum::CustomerCsvImport => proto::BatchJobType::CustomerCsvImport,
            BatchJobTypeEnum::SubscriptionCsvImport => proto::BatchJobType::SubscriptionCsvImport,
            BatchJobTypeEnum::SubscriptionPlanMigration => {
                proto::BatchJobType::SubscriptionPlanMigration
            }
            BatchJobTypeEnum::TaxReportExport => proto::BatchJobType::TaxReportExport,
        })
    }
}

impl From<BatchJobStatusWrapper> for BatchJobStatusEnum {
    fn from(val: BatchJobStatusWrapper) -> Self {
        match val.0 {
            proto::BatchJobStatus::Pending => BatchJobStatusEnum::Pending,
            proto::BatchJobStatus::Chunking => BatchJobStatusEnum::Chunking,
            proto::BatchJobStatus::Processing => BatchJobStatusEnum::Processing,
            proto::BatchJobStatus::Completed => BatchJobStatusEnum::Completed,
            proto::BatchJobStatus::CompletedWithErrors => BatchJobStatusEnum::CompletedWithErrors,
            proto::BatchJobStatus::Failed => BatchJobStatusEnum::Failed,
            proto::BatchJobStatus::Cancelled => BatchJobStatusEnum::Cancelled,
        }
    }
}

impl From<BatchJobStatusEnum> for BatchJobStatusWrapper {
    fn from(e: BatchJobStatusEnum) -> Self {
        Self(match e {
            BatchJobStatusEnum::Pending => proto::BatchJobStatus::Pending,
            BatchJobStatusEnum::Chunking => proto::BatchJobStatus::Chunking,
            BatchJobStatusEnum::Processing => proto::BatchJobStatus::Processing,
            BatchJobStatusEnum::Completed => proto::BatchJobStatus::Completed,
            BatchJobStatusEnum::CompletedWithErrors => proto::BatchJobStatus::CompletedWithErrors,
            BatchJobStatusEnum::Failed => proto::BatchJobStatus::Failed,
            BatchJobStatusEnum::Cancelled => proto::BatchJobStatus::Cancelled,
        })
    }
}

impl From<BatchJobChunkStatusEnum> for BatchJobChunkStatusWrapper {
    fn from(e: BatchJobChunkStatusEnum) -> Self {
        Self(match e {
            BatchJobChunkStatusEnum::Pending => proto::BatchJobChunkStatus::ChunkPending,
            BatchJobChunkStatusEnum::Processing => proto::BatchJobChunkStatus::ChunkProcessing,
            BatchJobChunkStatusEnum::Completed => proto::BatchJobChunkStatus::ChunkCompleted,
            BatchJobChunkStatusEnum::Failed => proto::BatchJobChunkStatus::ChunkFailed,
            BatchJobChunkStatusEnum::Skipped => proto::BatchJobChunkStatus::ChunkSkipped,
        })
    }
}

pub fn batch_job_to_proto(
    job: domain::BatchJob,
    jwt_secret: &SecretString,
    created_by_display_name: Option<String>,
) -> proto::BatchJob {
    let error_csv_token = if job.error_output_key.is_some() {
        generate_error_csv_token(job.id, job.tenant_id, jwt_secret).ok()
    } else {
        None
    };

    let input_file_token = if job.input_source_key.is_some() {
        generate_input_file_token(job.id, job.tenant_id, jwt_secret).ok()
    } else {
        None
    };

    let output_token = if job.output_key.is_some() {
        generate_output_token(job.id, job.tenant_id, jwt_secret).ok()
    } else {
        None
    };

    proto::BatchJob {
        id: job.id.to_string(),
        tenant_id: job.tenant_id.as_proto(),
        job_type: BatchJobTypeWrapper::from(job.job_type).0 as i32,
        status: BatchJobStatusWrapper::from(job.status).0 as i32,
        total_items: job.total_items,
        processed_items: job.processed_items,
        failed_items: job.failed_items,
        created_by: job.created_by.to_string(),
        created_at: job.created_at.as_proto(),
        completed_at: job.completed_at.map(|t| t.as_proto()),
        error_message: job.error_message,
        error_csv_token,
        input_file_name: job.input_file_name,
        created_by_display_name,
        input_file_token,
        output_token,
    }
}

pub fn batch_job_chunk_to_proto(chunk: domain::BatchJobChunk) -> proto::BatchJobChunk {
    let events = chunk
        .events
        .into_iter()
        .map(|e| proto::BatchJobChunkEvent {
            event: e.event,
            attempt: e.attempt,
            message: e.message,
            timestamp: e.timestamp.as_proto(),
        })
        .collect();

    proto::BatchJobChunk {
        id: chunk.id.to_string(),
        chunk_index: chunk.chunk_index,
        status: BatchJobChunkStatusWrapper::from(chunk.status).0 as i32,
        item_offset: chunk.item_offset,
        item_count: chunk.item_count,
        processed_count: chunk.processed_count,
        failed_count: chunk.failed_count,
        retry_count: chunk.retry_count,
        events,
        retry_after: chunk.retry_after.map(|t| t.as_proto()),
    }
}

pub fn batch_job_item_failure_to_proto(
    failure: domain::BatchJobItemFailure,
) -> proto::BatchJobItemFailure {
    proto::BatchJobItemFailure {
        id: failure.id.to_string(),
        chunk_id: failure.chunk_id.to_string(),
        item_index: failure.item_index,
        item_identifier: failure.item_identifier,
        reason: failure.reason,
    }
}
