use super::AppState;
use axum::Json;
use meteroid_rest_common::extract::{ApiJson, ApiPath, ApiValid};

use axum::Extension;
use axum::extract::State;
use axum::response::IntoResponse;
use common_domain::ids::ProductId;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_store::domain::ProductNew;
use meteroid_store::repositories::products::{ProductInterface, ProductUpdate};

use crate::products::mapping;
use crate::products::model::*;
use meteroid_app::errors::RestApiError;
use meteroid_app::features::ensure_credit_pricing_allowed;
use meteroid_rest_common::QueryParams;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_rest_common::model::{PaginationExt, validate_order_by};

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
pub async fn list_products(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    ApiValid(QueryParams(request)): ApiValid<QueryParams<ProductListRequest>>,
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

    let ids: Vec<ProductId> = res.items.iter().map(|p| p.id).collect();
    let mut facts = app_state
        .store
        .product_list_facts(authorized_state.tenant_id, &ids)
        .await
        .map_err(|e| {
            log::error!("Error handling list_products: {e}");
            RestApiError::StoreError
        })?;
    let data: Vec<Product> = res
        .items
        .into_iter()
        .map(|product| {
            let structures = facts
                .remove(&product.id)
                .map(|f| f.charge_structures)
                .unwrap_or_default();
            mapping::product_to_rest(product, &structures)
        })
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
pub async fn get_product(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    ApiPath(product_id): ApiPath<ProductId>,
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

    let structures = charge_structures(&app_state, authorized_state.tenant_id, product_id).await?;
    Ok(Json(mapping::product_to_rest(product, &structures)))
}

/// How the live charges using the product bill it.
async fn charge_structures(
    app_state: &AppState,
    tenant_id: common_domain::ids::TenantId,
    product_id: ProductId,
) -> Result<Vec<meteroid_store::domain::prices::FeeStructure>, RestApiError> {
    Ok(app_state
        .store
        .product_list_facts(tenant_id, &[product_id])
        .await
        .map_err(RestApiError::from)?
        .remove(&product_id)
        .map(|facts| facts.charge_structures)
        .unwrap_or_default())
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
pub async fn create_product(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<CreateProductRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    // `fee_structure` is not kept: the charges using the product own how it is billed. A credit
    // structure is still refused where credit pricing is not enabled.
    ensure_credit_pricing_allowed(
        app_state.feature_resolver.as_ref(),
        authorized_state.organization_id,
        matches!(payload.fee_structure, Some(ProductFeeStructure::Credit(_))),
    )
    .await?;
    let product = app_state
        .store
        .create_product(
            authorized_state.as_actor(),
            ProductNew {
                name: payload.name,
                description: payload.description,
                tenant_id: authorized_state.tenant_id,
                family_id: payload.product_family_id,
                catalog: payload.catalog,
                tax_category_id: None,
                icon: None,
                color: None,
            },
        )
        .await
        .map_err(|e| {
            log::error!("Error creating product: {e}");
            RestApiError::from(e)
        })?;

    Ok((
        StatusCode::CREATED,
        Json(mapping::product_to_rest(product, &[])),
    ))
}

// ── Update product ─────────────────────────────────────────────

/// Update a product
///
/// Partially update product fields. A `fee_structure` is ignored: the charges using the product
/// own how it is billed.
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
pub async fn update_product(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(product_id): ApiPath<ProductId>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<UpdateProductRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    // A credit structure is refused where credit pricing is not enabled, unless the product's
    // charges already sell credit.
    if matches!(payload.fee_structure, Some(ProductFeeStructure::Credit(_))) {
        let sells_credit = charge_structures(&app_state, authorized_state.tenant_id, product_id)
            .await?
            .iter()
            .any(meteroid_store::domain::prices::FeeStructure::is_credit);
        ensure_credit_pricing_allowed(
            app_state.feature_resolver.as_ref(),
            authorized_state.organization_id,
            !sells_credit,
        )
        .await?;
    }
    let product = app_state
        .store
        .update_product(
            authorized_state.as_actor(),
            ProductUpdate {
                id: product_id,
                tenant_id: authorized_state.tenant_id,
                name: payload.name,
                description: payload.description,
                tax_category_id: None,
                icon: None,
                color: None,
            },
        )
        .await
        .map_err(|e| {
            log::error!("Error updating product: {e}");
            RestApiError::from(e)
        })?;

    let structures = charge_structures(&app_state, authorized_state.tenant_id, product_id).await?;
    Ok(Json(mapping::product_to_rest(product, &structures)))
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
pub async fn archive_product(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(product_id): ApiPath<ProductId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .archive_product(
            authorized_state.as_actor(),
            product_id,
            authorized_state.tenant_id,
        )
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
pub async fn unarchive_product(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(product_id): ApiPath<ProductId>,
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
