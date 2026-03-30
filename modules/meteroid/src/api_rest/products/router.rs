use super::AppState;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::ProductId;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_store::domain::ProductNew;
use meteroid_store::repositories::products::{ProductInterface, ProductUpdate};

use crate::api_rest::QueryParams;
use crate::api_rest::error::RestErrorResponse;
use crate::api_rest::model::{PaginationExt, validate_order_by};
use crate::api_rest::products::mapping;
use crate::api_rest::products::model::*;
use crate::errors::RestApiError;

// ── List products ──────────────────────────────────────────────

/// List products
#[utoipa::path(
    get,
    tag = "Products",
    path = "/api/v1/products",
    params(ProductListRequest),
    responses(
        (status = 200, description = "List of products", body = ProductListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn list_products(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(QueryParams(request)): Valid<QueryParams<ProductListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let order_by = validate_order_by(&request.order_by, &["name", "created_at"], "name.asc")
        .map_err(RestApiError::InvalidInput)?;

    let res = match &request.search {
        Some(q) if !q.is_empty() => {
            app_state
                .store
                .search_products(
                    authorized_state.tenant_id,
                    request.product_family_id,
                    q,
                    false,
                    request.pagination.into(),
                    Some(order_by),
                )
                .await
        }
        _ => {
            app_state
                .store
                .list_products(
                    authorized_state.tenant_id,
                    request.product_family_id,
                    false,
                    request.pagination.into(),
                    Some(order_by),
                )
                .await
        }
    }
    .map_err(|e| {
        log::error!("Error handling list_products: {e}");
        RestApiError::StoreError
    })?;

    let data: Vec<Product> = res
        .items
        .into_iter()
        .map(mapping::product_to_rest)
        .collect();

    Ok(Json(ProductListResponse {
        data,
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

// ── Get product ────────────────────────────────────────────────

/// Get product details
#[utoipa::path(
    get,
    tag = "Products",
    path = "/api/v1/products/{product_id}",
    params(("product_id" = ProductId, Path, description = "Product ID")),
    responses(
        (status = 200, description = "Product details", body = Product),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Product not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn get_product(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(product_id): Path<ProductId>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let product = app_state
        .store
        .find_product_by_id(product_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error fetching product: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::product_to_rest(product)))
}

// ── Create product ─────────────────────────────────────────────

/// Create a product
#[utoipa::path(
    post,
    tag = "Products",
    path = "/api/v1/products",
    request_body(content = CreateProductRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Product created", body = Product),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn create_product(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<CreateProductRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let (fee_type, fee_structure) = mapping::rest_fee_structure_to_domain(&payload.fee_structure)?;

    let product = app_state
        .store
        .create_product(ProductNew {
            name: payload.name,
            description: payload.description,
            created_by: authorized_state.actor_id,
            tenant_id: authorized_state.tenant_id,
            family_id: payload.product_family_id,
            fee_type,
            fee_structure,
            catalog: payload.catalog,
        })
        .await
        .map_err(|e| {
            log::error!("Error creating product: {e}");
            RestApiError::from(e)
        })?;

    Ok((StatusCode::CREATED, Json(mapping::product_to_rest(product))))
}

// ── Update product ─────────────────────────────────────────────

/// Update a product
///
/// Partially update product fields. The fee_type is immutable and cannot be changed.
#[utoipa::path(
    patch,
    tag = "Products",
    path = "/api/v1/products/{product_id}",
    params(("product_id" = ProductId, Path, description = "Product ID")),
    request_body(content = UpdateProductRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Product updated", body = Product),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Product not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn update_product(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(product_id): Path<ProductId>,
    Valid(Json(payload)): Valid<Json<UpdateProductRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let (fee_type, fee_structure) = match &payload.fee_structure {
        Some(fs) => {
            let (ft, fst) = mapping::rest_fee_structure_to_domain(fs)?;
            (Some(ft), Some(fst))
        }
        None => (None, None),
    };

    let product = app_state
        .store
        .update_product(ProductUpdate {
            id: product_id,
            tenant_id: authorized_state.tenant_id,
            name: payload.name,
            description: payload.description,
            fee_type,
            fee_structure,
        })
        .await
        .map_err(|e| {
            log::error!("Error updating product: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::product_to_rest(product)))
}

// ── Archive product ────────────────────────────────────────────

/// Archive a product
#[utoipa::path(
    post,
    tag = "Products",
    path = "/api/v1/products/{product_id}/archive",
    params(("product_id" = ProductId, Path, description = "Product ID")),
    responses(
        (status = 204, description = "Product archived"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Product not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn archive_product(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(product_id): Path<ProductId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .archive_product(product_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error archiving product: {e}");
            RestApiError::from(e)
        })
        .map(|_| StatusCode::NO_CONTENT)
}

/// Unarchive a product
#[utoipa::path(
    post,
    tag = "Products",
    path = "/api/v1/products/{product_id}/unarchive",
    params(("product_id" = ProductId, Path, description = "Product ID")),
    responses(
        (status = 204, description = "Product unarchived"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Product not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn unarchive_product(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(product_id): Path<ProductId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .unarchive_product(product_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error unarchiving product: {e}");
            RestApiError::from(e)
        })
        .map(|_| StatusCode::NO_CONTENT)
}
