use super::AppState;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::AddOnId;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_store::domain::add_ons::{AddOnNew, AddOnPatch};
use meteroid_store::domain::price_components::PriceEntry;
use meteroid_store::repositories::add_ons::AddOnInterface;

use crate::api_rest::QueryParams;
use crate::api_rest::addons::mapping;
use crate::api_rest::addons::model::*;
use crate::api_rest::error::RestErrorResponse;
use crate::api_rest::model::PaginationExt;
use crate::errors::RestApiError;

// ── List add-ons ──────────────────────────────────────────────

/// List add-ons
#[utoipa::path(
    get,
    tag = "Add-ons",
    path = "/api/v1/addons",
    params(AddOnListRequest),
    responses(
        (status = 200, description = "List of add-ons", body = AddOnListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn list_addons(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(QueryParams(request)): Valid<QueryParams<AddOnListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .list_add_ons(
            authorized_state.tenant_id,
            None,
            request.pagination.into(),
            request.search,
            request.currency,
            request.include_archived,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_addons: {e}");
            RestApiError::StoreError
        })?;

    let data: Vec<AddOn> = res.items.into_iter().map(mapping::addon_to_rest).collect();

    Ok(Json(AddOnListResponse {
        data,
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

// ── Get add-on ────────────────────────────────────────────────

/// Get add-on details
#[utoipa::path(
    get,
    tag = "Add-ons",
    path = "/api/v1/addons/{addon_id}",
    params(("addon_id" = AddOnId, Path, description = "Add-on ID")),
    responses(
        (status = 200, description = "Add-on details", body = AddOn),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Add-on not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn get_addon(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(addon_id): Path<AddOnId>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let addon = app_state
        .store
        .get_add_on_by_id(authorized_state.tenant_id, addon_id)
        .await
        .map_err(|e| {
            log::error!("Error fetching add-on: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::addon_to_rest(addon)))
}

// ── Create add-on ─────────────────────────────────────────────

/// Create an add-on
#[utoipa::path(
    post,
    tag = "Add-ons",
    path = "/api/v1/addons",
    request_body(content = CreateAddOnRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Add-on created", body = AddOn),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn create_addon(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<CreateAddOnRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let addon = app_state
        .store
        .create_add_on(AddOnNew {
            name: payload.name,
            tenant_id: authorized_state.tenant_id,
            product_id: payload.product_id,
            price_id: payload.price_id,
            description: payload.description,
            self_serviceable: payload.self_serviceable,
            max_instances_per_subscription: payload.max_instances_per_subscription,
        })
        .await
        .map_err(|e| {
            log::error!("Error creating add-on: {e}");
            RestApiError::from(e)
        })?;

    Ok((StatusCode::CREATED, Json(mapping::addon_to_rest(addon))))
}

// ── Update add-on ─────────────────────────────────────────────

/// Update an add-on
#[utoipa::path(
    patch,
    tag = "Add-ons",
    path = "/api/v1/addons/{addon_id}",
    params(("addon_id" = AddOnId, Path, description = "Add-on ID")),
    request_body(content = UpdateAddOnRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Add-on updated", body = AddOn),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Add-on not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn update_addon(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(addon_id): Path<AddOnId>,
    Valid(Json(payload)): Valid<Json<UpdateAddOnRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let price_entry = payload.price_id.map(PriceEntry::Existing);

    let addon = app_state
        .store
        .update_add_on(
            AddOnPatch {
                id: addon_id,
                tenant_id: authorized_state.tenant_id,
                name: payload.name,
                description: payload.description,
                self_serviceable: payload.self_serviceable,
                max_instances_per_subscription: payload.max_instances_per_subscription,
            },
            price_entry,
            authorized_state.actor_id,
        )
        .await
        .map_err(|e| {
            log::error!("Error updating add-on: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::addon_to_rest(addon)))
}

// ── Archive add-on ────────────────────────────────────────────

/// Archive an add-on
#[utoipa::path(
    post,
    tag = "Add-ons",
    path = "/api/v1/addons/{addon_id}/archive",
    params(("addon_id" = AddOnId, Path, description = "Add-on ID")),
    responses(
        (status = 204, description = "Add-on archived"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Add-on not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn archive_addon(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(addon_id): Path<AddOnId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .archive_add_on(addon_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error archiving add-on: {e}");
            RestApiError::from(e)
        })
        .map(|_| StatusCode::NO_CONTENT)
}

/// Unarchive an add-on
#[utoipa::path(
    post,
    tag = "Add-ons",
    path = "/api/v1/addons/{addon_id}/unarchive",
    params(("addon_id" = AddOnId, Path, description = "Add-on ID")),
    responses(
        (status = 204, description = "Add-on unarchived"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Add-on not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn unarchive_addon(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(addon_id): Path<AddOnId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .unarchive_add_on(addon_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error unarchiving add-on: {e}");
            RestApiError::from(e)
        })
        .map(|_| StatusCode::NO_CONTENT)
}
