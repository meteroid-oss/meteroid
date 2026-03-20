use super::AppState;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::BillableMetricId;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_store::domain;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;

use crate::api_rest::QueryParams;
use crate::api_rest::error::RestErrorResponse;
use crate::api_rest::metrics::mapping;
use crate::api_rest::metrics::model::*;
use crate::api_rest::model::PaginationExt;
use crate::errors::RestApiError;

// ── List metrics ───────────────────────────────────────────────

/// List billable metrics
#[utoipa::path(
    get,
    tag = "Metrics",
    path = "/api/v1/metrics",
    params(MetricListRequest),
    responses(
        (status = 200, description = "List of metrics", body = MetricListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn list_metrics(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(QueryParams(request)): Valid<QueryParams<MetricListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .list_billable_metrics(
            authorized_state.tenant_id,
            request.pagination.into(),
            request.product_family_id,
            None,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_metrics: {e}");
            RestApiError::StoreError
        })?;

    let data: Vec<MetricSummary> = res
        .items
        .into_iter()
        .map(mapping::metric_meta_to_rest)
        .collect();

    Ok(Json(MetricListResponse {
        data,
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

// ── Get metric ─────────────────────────────────────────────────

/// Get metric details
#[utoipa::path(
    get,
    tag = "Metrics",
    path = "/api/v1/metrics/{metric_id}",
    params(("metric_id" = BillableMetricId, Path, description = "Metric ID")),
    responses(
        (status = 200, description = "Metric details", body = Metric),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Metric not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn get_metric(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(metric_id): Path<BillableMetricId>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let metric = app_state
        .store
        .find_billable_metric_by_id(metric_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error fetching metric: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::metric_to_rest(metric)))
}

// ── Create metric ──────────────────────────────────────────────

/// Create a billable metric
#[utoipa::path(
    post,
    tag = "Metrics",
    path = "/api/v1/metrics",
    request_body(content = CreateMetricRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Metric created", body = Metric),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn create_metric(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<CreateMetricRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let (unit_conversion_factor, unit_conversion_rounding) = match payload.unit_conversion {
        Some(uc) => (Some(uc.factor), Some(uc.rounding.into())),
        None => (None, None),
    };

    let metric = app_state
        .store
        .insert_billable_metric(domain::BillableMetricNew {
            name: payload.name,
            description: payload.description,
            code: payload.code,
            aggregation_type: payload.aggregation_type.into(),
            aggregation_key: payload.aggregation_key,
            unit_conversion_factor,
            unit_conversion_rounding,
            segmentation_matrix: payload.segmentation_matrix.map(Into::into),
            usage_group_key: payload.usage_group_key,
            created_by: authorized_state.actor_id,
            tenant_id: authorized_state.tenant_id,
            product_family_id: payload.product_family_id,
            product_id: payload.product_id,
        })
        .await
        .map_err(|e| {
            log::error!("Error creating metric: {e}");
            RestApiError::from(e)
        })?;

    Ok((StatusCode::CREATED, Json(mapping::metric_to_rest(metric))))
}

// ── Update metric ──────────────────────────────────────────────

/// Update a billable metric
///
/// Partially update metric fields. Code and aggregation_type are immutable.
#[utoipa::path(
    patch,
    tag = "Metrics",
    path = "/api/v1/metrics/{metric_id}",
    params(("metric_id" = BillableMetricId, Path, description = "Metric ID")),
    request_body(content = UpdateMetricRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Metric updated", body = Metric),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Metric not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn update_metric(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(metric_id): Path<BillableMetricId>,
    Valid(Json(payload)): Valid<Json<UpdateMetricRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let (unit_conversion_factor, unit_conversion_rounding) = match payload.unit_conversion {
        Some(opt_uc) => match opt_uc {
            Some(uc) => (Some(Some(uc.factor)), Some(Some(uc.rounding.into()))),
            None => (Some(None), Some(None)),
        },
        None => (None, None),
    };

    let metric = app_state
        .store
        .update_billable_metric(
            metric_id,
            authorized_state.tenant_id,
            domain::BillableMetricUpdate {
                name: payload.name,
                description: payload.description,
                unit_conversion_factor,
                unit_conversion_rounding,
                segmentation_matrix: payload.segmentation_matrix.map(|opt| opt.map(Into::into)),
            },
        )
        .await
        .map_err(|e| {
            log::error!("Error updating metric: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::metric_to_rest(metric)))
}

// ── Archive / Unarchive ────────────────────────────────────────

/// Archive a billable metric
#[utoipa::path(
    post,
    tag = "Metrics",
    path = "/api/v1/metrics/{metric_id}/archive",
    params(("metric_id" = BillableMetricId, Path, description = "Metric ID")),
    responses(
        (status = 204, description = "Metric archived"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Metric not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn archive_metric(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(metric_id): Path<BillableMetricId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .archive_billable_metric(metric_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error archiving metric: {e}");
            RestApiError::from(e)
        })
        .map(|_| StatusCode::NO_CONTENT)
}

/// Unarchive a billable metric
#[utoipa::path(
    post,
    tag = "Metrics",
    path = "/api/v1/metrics/{metric_id}/unarchive",
    params(("metric_id" = BillableMetricId, Path, description = "Metric ID")),
    responses(
        (status = 204, description = "Metric unarchived"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Metric not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn unarchive_metric(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(metric_id): Path<BillableMetricId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .unarchive_billable_metric(metric_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error unarchiving metric: {e}");
            RestApiError::from(e)
        })
        .map(|_| StatusCode::NO_CONTENT)
}
