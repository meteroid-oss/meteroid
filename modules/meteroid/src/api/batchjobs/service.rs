use std::str::FromStr;

use bytes::Bytes;
use common_domain::ids::{BatchJobChunkId, BatchJobId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::batchjobs::v1 as proto;
use meteroid_grpc::meteroid::api::batchjobs::v1::batch_jobs_service_server::BatchJobsService;
use meteroid_grpc::meteroid::api::batchjobs::v1::*;
use meteroid_store::domain::batch_jobs::BatchJobNew;
use meteroid_store::repositories::batch_jobs::BatchJobsInterface;
use meteroid_store::repositories::users::UserInterface;
use sha2::{Digest, Sha256};
use tonic::{Request, Response, Status};

use super::BatchJobsServiceComponents;
use super::error::BatchJobApiError;
use super::mapping::{self, BatchJobStatusWrapper, BatchJobTypeWrapper};
use crate::services::storage::Prefix;

#[tonic::async_trait]
impl BatchJobsService for BatchJobsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn create_batch_job(
        &self,
        request: Request<CreateBatchJobRequest>,
    ) -> Result<Response<CreateBatchJobResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;
        let req = request.into_inner();

        let job_type: meteroid_store::domain::enums::BatchJobTypeEnum =
            BatchJobTypeWrapper(req.job_type()).into();

        // Store file in S3 if provided
        const MAX_CSV_SIZE: usize = 10 * 1024 * 1024; // 10MB

        let (input_source_key, file_hash) =
            if let Some(file_data) = req.file_data.filter(|d| !d.is_empty()) {
                if file_data.len() > MAX_CSV_SIZE {
                    return Err(BatchJobApiError::InvalidArgument(format!(
                        "File size ({:.1} MB) exceeds maximum allowed ({} MB)",
                        file_data.len() as f64 / 1_048_576.0,
                        MAX_CSV_SIZE / 1_048_576
                    ))
                    .into());
                }

                let data = Bytes::from(file_data);

                let hash = format!("{:x}", Sha256::digest(&data));

                // Check for active duplicate (Pending/Chunking/Processing) — return existing job
                if let Some(existing) = self
                    .store
                    .check_duplicate_batch_job(tenant_id, job_type.clone(), &hash)
                    .await
                    .map_err(BatchJobApiError::from)?
                {
                    return Ok(Response::new(CreateBatchJobResponse {
                        job: Some(mapping::batch_job_to_proto(
                            existing,
                            &self.jwt_secret,
                            None,
                        )),
                    }));
                }

                // Check for completed duplicate — reject unless force_duplicate
                if !req.force_duplicate
                    && let Some(existing) = self
                        .store
                        .check_completed_duplicate_batch_job(tenant_id, job_type.clone(), &hash)
                        .await
                        .map_err(BatchJobApiError::from)?
                {
                    return Err(BatchJobApiError::DuplicateImport(format!(
                        "This file was already imported (job {})",
                        existing.id
                    ))
                    .into());
                }

                let prefix = Prefix::BatchJobInput { tenant_id };
                let doc_id = self
                    .object_store
                    .store(data, prefix)
                    .await
                    .map_err(|e| BatchJobApiError::ObjectStoreError(format!("{e:?}")))?;

                (Some(doc_id.to_string()), Some(hash))
            } else {
                (None, None)
            };

        let input_params =
            if req.params.is_empty() {
                None
            } else {
                Some(serde_json::to_value(&req.params).map_err(|e| {
                    BatchJobApiError::InvalidArgument(format!("Invalid params: {e}"))
                })?)
            };

        if input_source_key.is_none() && input_params.is_none() {
            return Err(BatchJobApiError::InvalidArgument(
                "Either file_data or params must be provided".to_string(),
            )
            .into());
        }

        let input_file_name = req.file_name.filter(|s| !s.is_empty());

        let job = self
            .store
            .create_batch_job(BatchJobNew {
                tenant_id,
                job_type,
                input_source_key,
                input_params,
                file_hash,
                created_by: actor,
                input_file_name,
            })
            .await
            .map_err(BatchJobApiError::from)?;

        Ok(Response::new(CreateBatchJobResponse {
            job: Some(mapping::batch_job_to_proto(job, &self.jwt_secret, None)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_batch_job(
        &self,
        request: Request<GetBatchJobRequest>,
    ) -> Result<Response<GetBatchJobResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let job_id = BatchJobId::from_str(&req.job_id)
            .map_err(|_| Status::invalid_argument("Invalid job ID"))?;

        let detail = self
            .store
            .get_batch_job(job_id, tenant_id)
            .await
            .map_err(BatchJobApiError::from)?;

        let display_name = self
            .store
            .find_user_by_id_and_tenant(detail.job.created_by, tenant_id)
            .await
            .ok()
            .map(
                |u| match (u.first_name.as_deref(), u.last_name.as_deref()) {
                    (Some(f), Some(l)) => format!("{f} {l}"),
                    (Some(f), None) => f.to_string(),
                    _ => u.email,
                },
            );

        Ok(Response::new(GetBatchJobResponse {
            job: Some(mapping::batch_job_to_proto(
                detail.job,
                &self.jwt_secret,
                display_name,
            )),
            chunks: detail
                .chunks
                .into_iter()
                .map(mapping::batch_job_chunk_to_proto)
                .collect(),
            failure_count: detail.failure_count,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_batch_jobs(
        &self,
        request: Request<ListBatchJobsRequest>,
    ) -> Result<Response<ListBatchJobsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let job_type = req
            .job_type
            .map(|jt| proto::BatchJobType::try_from(jt).map(|t| BatchJobTypeWrapper(t).into()))
            .transpose()
            .map_err(|_| Status::invalid_argument("Invalid job_type"))?;

        let statuses = if req.statuses.is_empty() {
            None
        } else {
            let parsed: Result<Vec<_>, _> = req
                .statuses
                .into_iter()
                .map(|s| {
                    proto::BatchJobStatus::try_from(s).map(|s| BatchJobStatusWrapper(s).into())
                })
                .collect();
            Some(parsed.map_err(|_| Status::invalid_argument("Invalid status"))?)
        };

        let per_page = req.limit.clamp(1, 100) as u32;
        let page = (req.offset.max(0) as u32) / per_page;

        let pagination = meteroid_store::domain::PaginationRequest {
            page,
            per_page: Some(per_page),
        };

        let res = self
            .store
            .list_batch_jobs(tenant_id, job_type, statuses, pagination)
            .await
            .map_err(BatchJobApiError::from)?;

        Ok(Response::new(ListBatchJobsResponse {
            jobs: res
                .items
                .into_iter()
                .map(|j| mapping::batch_job_to_proto(j, &self.jwt_secret, None))
                .collect(),
            total_count: res.total_results as i64,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn retry_batch_job(
        &self,
        request: Request<RetryBatchJobRequest>,
    ) -> Result<Response<RetryBatchJobResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let job_id = BatchJobId::from_str(&req.job_id)
            .map_err(|_| Status::invalid_argument("Invalid job ID"))?;

        let retried = self
            .store
            .retry_failed_chunks(job_id, tenant_id)
            .await
            .map_err(BatchJobApiError::from)?;

        Ok(Response::new(RetryBatchJobResponse {
            retried_chunks: retried,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn cancel_batch_job(
        &self,
        request: Request<CancelBatchJobRequest>,
    ) -> Result<Response<CancelBatchJobResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let job_id = BatchJobId::from_str(&req.job_id)
            .map_err(|_| Status::invalid_argument("Invalid job ID"))?;

        self.store
            .cancel_batch_job(job_id, tenant_id)
            .await
            .map_err(BatchJobApiError::from)?;

        Ok(Response::new(CancelBatchJobResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn list_batch_job_failures(
        &self,
        request: Request<ListBatchJobFailuresRequest>,
    ) -> Result<Response<ListBatchJobFailuresResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let job_id = BatchJobId::from_str(&req.job_id)
            .map_err(|_| Status::invalid_argument("Invalid job ID"))?;

        let chunk_id = req
            .chunk_id
            .map(|id| {
                BatchJobChunkId::from_str(&id)
                    .map_err(|_| Status::invalid_argument("Invalid chunk ID"))
            })
            .transpose()?;

        let limit = req.limit.clamp(1, 100) as i64;
        let offset = req.offset.max(0) as i64;

        let failures = self
            .store
            .list_batch_job_failures(job_id, tenant_id, chunk_id, limit, offset)
            .await
            .map_err(BatchJobApiError::from)?;

        let total_count = self
            .store
            .count_batch_job_failures(job_id, tenant_id, chunk_id)
            .await
            .map_err(BatchJobApiError::from)?;

        Ok(Response::new(ListBatchJobFailuresResponse {
            failures: failures
                .into_iter()
                .map(mapping::batch_job_item_failure_to_proto)
                .collect(),
            total_count,
        }))
    }
}
