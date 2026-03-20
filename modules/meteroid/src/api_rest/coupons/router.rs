use super::AppState;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::CouponId;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_store::domain::coupons::{CouponNew, CouponPatch, CouponStatusPatch};
use meteroid_store::repositories::coupons::CouponInterface;

use crate::api_rest::QueryParams;
use crate::api_rest::coupons::mapping;
use crate::api_rest::coupons::model::*;
use crate::api_rest::error::RestErrorResponse;
use crate::api_rest::model::PaginationExt;
use crate::errors::RestApiError;

// ── List coupons ──────────────────────────────────────────────

/// List coupons
#[utoipa::path(
    get,
    tag = "Coupons",
    path = "/api/v1/coupons",
    params(CouponListRequest),
    responses(
        (status = 200, description = "List of coupons", body = CouponListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn list_coupons(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(QueryParams(request)): Valid<QueryParams<CouponListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let filter = match request.filter {
        Some(CouponFilterEnum::All) | None => meteroid_store::domain::coupons::CouponFilter::ALL,
        Some(CouponFilterEnum::Active) => meteroid_store::domain::coupons::CouponFilter::ACTIVE,
        Some(CouponFilterEnum::Inactive) => meteroid_store::domain::coupons::CouponFilter::INACTIVE,
        Some(CouponFilterEnum::Archived) => meteroid_store::domain::coupons::CouponFilter::ARCHIVED,
    };

    let res = app_state
        .store
        .list_coupons(
            authorized_state.tenant_id,
            request.pagination.into(),
            request.search.clone(),
            filter,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_coupons: {e}");
            RestApiError::StoreError
        })?;

    let data: Vec<Coupon> = res.items.into_iter().map(mapping::coupon_to_rest).collect();

    Ok(Json(CouponListResponse {
        data,
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

// ── Get coupon ────────────────────────────────────────────────

/// Get coupon details
#[utoipa::path(
    get,
    tag = "Coupons",
    path = "/api/v1/coupons/{coupon_id}",
    params(("coupon_id" = CouponId, Path, description = "Coupon ID")),
    responses(
        (status = 200, description = "Coupon details", body = Coupon),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Coupon not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn get_coupon(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(coupon_id): Path<CouponId>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let coupon = app_state
        .store
        .get_coupon_by_id(authorized_state.tenant_id, coupon_id)
        .await
        .map_err(|e| {
            log::error!("Error fetching coupon: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::coupon_to_rest(coupon)))
}

// ── Create coupon ─────────────────────────────────────────────

/// Create a coupon
#[utoipa::path(
    post,
    tag = "Coupons",
    path = "/api/v1/coupons",
    request_body(content = CreateCouponRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Coupon created", body = Coupon),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn create_coupon(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<CreateCouponRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let discount = mapping::rest_discount_to_domain(&payload.discount)?;

    let coupon = app_state
        .store
        .create_coupon(CouponNew {
            code: payload.code,
            description: payload.description.unwrap_or_default(),
            tenant_id: authorized_state.tenant_id,
            discount,
            expires_at: payload.expires_at,
            redemption_limit: payload.redemption_limit,
            recurring_value: payload.recurring_value,
            reusable: payload.reusable,
            plan_ids: payload.plan_ids,
        })
        .await
        .map_err(|e| {
            log::error!("Error creating coupon: {e}");
            RestApiError::from(e)
        })?;

    Ok((StatusCode::CREATED, Json(mapping::coupon_to_rest(coupon))))
}

// ── Update coupon ─────────────────────────────────────────────

/// Update a coupon
#[utoipa::path(
    patch,
    tag = "Coupons",
    path = "/api/v1/coupons/{coupon_id}",
    params(("coupon_id" = CouponId, Path, description = "Coupon ID")),
    request_body(content = UpdateCouponRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Coupon updated", body = Coupon),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Coupon not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn update_coupon(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(coupon_id): Path<CouponId>,
    Valid(Json(payload)): Valid<Json<UpdateCouponRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let discount = payload
        .discount
        .as_ref()
        .map(mapping::rest_discount_to_domain)
        .transpose()?;

    let coupon = app_state
        .store
        .update_coupon(CouponPatch {
            id: coupon_id,
            tenant_id: authorized_state.tenant_id,
            description: payload.description,
            discount,
            plan_ids: payload.plan_ids,
        })
        .await
        .map_err(|e| {
            log::error!("Error updating coupon: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::coupon_to_rest(coupon)))
}

// ── Archive coupon ────────────────────────────────────────────

/// Archive a coupon
#[utoipa::path(
    post,
    tag = "Coupons",
    path = "/api/v1/coupons/{coupon_id}/archive",
    params(("coupon_id" = CouponId, Path, description = "Coupon ID")),
    responses(
        (status = 204, description = "Coupon archived"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Coupon not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn archive_coupon(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(coupon_id): Path<CouponId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .update_coupon_status(CouponStatusPatch {
            id: coupon_id,
            tenant_id: authorized_state.tenant_id,
            archived_at: Some(Some(chrono::Utc::now().naive_utc())),
            disabled: None,
        })
        .await
        .map_err(|e| {
            log::error!("Error archiving coupon: {e}");
            RestApiError::from(e)
        })
        .map(|_| StatusCode::NO_CONTENT)
}

/// Unarchive a coupon
#[utoipa::path(
    post,
    tag = "Coupons",
    path = "/api/v1/coupons/{coupon_id}/unarchive",
    params(("coupon_id" = CouponId, Path, description = "Coupon ID")),
    responses(
        (status = 204, description = "Coupon unarchived"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Coupon not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn unarchive_coupon(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(coupon_id): Path<CouponId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .update_coupon_status(CouponStatusPatch {
            id: coupon_id,
            tenant_id: authorized_state.tenant_id,
            archived_at: Some(None),
            disabled: None,
        })
        .await
        .map_err(|e| {
            log::error!("Error unarchiving coupon: {e}");
            RestApiError::from(e)
        })
        .map(|_| StatusCode::NO_CONTENT)
}

// ── Disable coupon ────────────────────────────────────────────

/// Disable a coupon
#[utoipa::path(
    post,
    tag = "Coupons",
    path = "/api/v1/coupons/{coupon_id}/disable",
    params(("coupon_id" = CouponId, Path, description = "Coupon ID")),
    responses(
        (status = 204, description = "Coupon disabled"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Coupon not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn disable_coupon(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(coupon_id): Path<CouponId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .update_coupon_status(CouponStatusPatch {
            id: coupon_id,
            tenant_id: authorized_state.tenant_id,
            archived_at: None,
            disabled: Some(true),
        })
        .await
        .map_err(|e| {
            log::error!("Error disabling coupon: {e}");
            RestApiError::from(e)
        })
        .map(|_| StatusCode::NO_CONTENT)
}

// ── Enable coupon ─────────────────────────────────────────────

/// Enable a coupon
#[utoipa::path(
    post,
    tag = "Coupons",
    path = "/api/v1/coupons/{coupon_id}/enable",
    params(("coupon_id" = CouponId, Path, description = "Coupon ID")),
    responses(
        (status = 204, description = "Coupon enabled"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Coupon not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn enable_coupon(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(coupon_id): Path<CouponId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .update_coupon_status(CouponStatusPatch {
            id: coupon_id,
            tenant_id: authorized_state.tenant_id,
            archived_at: None,
            disabled: Some(false),
        })
        .await
        .map_err(|e| {
            log::error!("Error enabling coupon: {e}");
            RestApiError::from(e)
        })
        .map(|_| StatusCode::NO_CONTENT)
}
