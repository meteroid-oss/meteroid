use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::BatchJobId;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::repositories::batch_jobs::BatchJobsInterface;

use crate::api_rest::AppState;
use crate::api_rest::QueryParams;
use crate::api_rest::batch_jobs::model::*;
use crate::api_rest::error::RestErrorResponse;
use crate::api_rest::model::PaginationExt;
use crate::errors::RestApiError;

/// List batch jobs
///
/// List batch jobs with optional filtering by type and status.
#[utoipa::path(
    get,
    tag = "Batch Jobs",
    path = "/api/v1/batch-jobs",
    params(
        BatchJobListRequest
    ),
    responses(
        (status = 200, description = "List of batch jobs", body = BatchJobListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn list_batch_jobs(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(QueryParams(request)): Valid<QueryParams<BatchJobListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let job_type = request.job_type.map(Into::into);
    let status = request
        .status
        .map(|v| v.into_iter().map(Into::into).collect());

    let res = app_state
        .store
        .list_batch_jobs(
            authorized_state.tenant_id,
            job_type,
            status,
            request.pagination.into(),
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_batch_jobs: {e}");
            RestApiError::from(e)
        })?;

    let data = res.items.into_iter().map(map_job).collect();

    Ok(Json(BatchJobListResponse {
        data,
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

/// Get batch job detail
///
/// Retrieve a single batch job with its chunks and failures.
#[utoipa::path(
    get,
    tag = "Batch Jobs",
    path = "/api/v1/batch-jobs/{batch_job_id}",
    params(
        ("batch_job_id" = BatchJobId, Path, description = "Batch job ID")
    ),
    responses(
        (status = 200, description = "Batch job detail", body = BatchJobDetailResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Batch job not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn get_batch_job(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(batch_job_id): Path<BatchJobId>,
) -> Result<impl IntoResponse, RestApiError> {
    let detail = app_state
        .store
        .get_batch_job(batch_job_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error handling get_batch_job: {e}");
            RestApiError::from(e)
        })?;

    let has_error_csv = detail.job.error_output_key.is_some();
    let error_csv_url = if has_error_csv {
        crate::api::batchjobs::mapping::generate_error_csv_url(
            detail.job.id,
            authorized_state.tenant_id,
            &app_state.jwt_secret,
        )
        .ok()
    } else {
        None
    };

    let input_file_url = if detail.job.input_source_key.is_some() {
        crate::api::batchjobs::mapping::generate_input_file_token(
            detail.job.id,
            authorized_state.tenant_id,
            &app_state.jwt_secret,
        )
        .ok()
        .map(|token| {
            format!(
                "/files/v1/batch-job/input/{}?token={}",
                detail.job.id, token
            )
        })
    } else {
        None
    };

    let response = BatchJobDetailResponse {
        id: detail.job.id,
        job_type: detail.job.job_type.into(),
        status: detail.job.status.into(),
        total_items: detail.job.total_items,
        processed_items: detail.job.processed_items,
        failed_items: detail.job.failed_items,
        created_by: detail.job.created_by,
        created_at: detail.job.created_at,
        completed_at: detail.job.completed_at,
        failure_count: detail.failure_count,
        has_error_csv,
        error_csv_url,
        input_file_name: detail.job.input_file_name,
        input_file_url,
    };

    Ok(Json(response))
}

/// List batch job failures
///
/// Retrieve paginated failures for a batch job.
#[utoipa::path(
    get,
    tag = "Batch Jobs",
    path = "/api/v1/batch-jobs/{batch_job_id}/failures",
    params(
        ("batch_job_id" = BatchJobId, Path, description = "Batch job ID"),
        BatchJobFailuresRequest
    ),
    responses(
        (status = 200, description = "List of failures", body = BatchJobFailuresResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Batch job not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn list_batch_job_failures(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(batch_job_id): Path<BatchJobId>,
    Valid(QueryParams(request)): Valid<QueryParams<BatchJobFailuresRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let tenant_id = authorized_state.tenant_id;

    let failures = app_state
        .store
        .list_batch_job_failures(
            batch_job_id,
            tenant_id,
            request.chunk_id,
            request.limit as i64,
            request.offset as i64,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_batch_job_failures: {e}");
            RestApiError::from(e)
        })?;

    let total_count = app_state
        .store
        .count_batch_job_failures(batch_job_id, tenant_id, request.chunk_id)
        .await
        .map_err(|e| {
            log::error!("Error handling count_batch_job_failures: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(BatchJobFailuresResponse {
        data: failures.into_iter().map(map_failure).collect(),
        total_count,
    }))
}

// --- Mapping helpers ---

fn map_job(job: meteroid_store::domain::batch_jobs::BatchJob) -> BatchJobResponse {
    BatchJobResponse {
        id: job.id,
        job_type: job.job_type.into(),
        status: job.status.into(),
        total_items: job.total_items,
        processed_items: job.processed_items,
        failed_items: job.failed_items,
        created_by: job.created_by,
        created_at: job.created_at,
        completed_at: job.completed_at,
        input_file_name: job.input_file_name,
    }
}

fn map_failure(
    f: meteroid_store::domain::batch_jobs::BatchJobItemFailure,
) -> BatchJobItemFailureResponse {
    BatchJobItemFailureResponse {
        id: f.id,
        chunk_id: f.chunk_id,
        item_index: f.item_index,
        item_identifier: f.item_identifier,
        reason: f.reason,
    }
}
