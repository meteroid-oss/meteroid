use super::AppState;

use axum::{Json, extract::State, response::IntoResponse};

use crate::api_rest::model::PaginationExt;
use crate::api_rest::productfamilies::mapping::{create_req_to_domain, domain_to_rest};
use crate::api_rest::productfamilies::model::{
    ProductFamily, ProductFamilyCreateRequest, ProductFamilyListRequest, ProductFamilyListResponse,
};
use crate::errors::RestApiError;
use axum::Extension;
use axum::extract::{Path, Query};
use axum_valid::Valid;
use common_domain::ids::{AliasOr, ProductFamilyId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_store::domain::OrderByRequest;
use meteroid_store::repositories::ProductFamilyInterface;

/// List product families
#[utoipa::path(
    get,
    tag = "Product Families",
    path = "/api/v1/product_families",
    params(
        ProductFamilyListRequest
    ),
    responses(
        (status = 200, description = "List of product families", body = ProductFamilyListResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error"),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn list_product_families(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(Query(request)): Valid<Query<ProductFamilyListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .list_product_families(
            authorized_state.tenant_id,
            request.pagination.into(),
            OrderByRequest::IdAsc,
            None,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_product_families: {}", e);
            RestApiError::StoreError
        })?;

    let items = res.items.into_iter().map(domain_to_rest).collect();

    Ok(Json(ProductFamilyListResponse {
        data: items,
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

/// Create product family
#[utoipa::path(
    post,
    tag = "Product Families",
    path = "/api/v1/product_families",
    request_body(content = ProductFamilyCreateRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Customer successfully created", body = ProductFamily),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error"),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn create_product_family(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<ProductFamilyCreateRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .insert_product_family(
            create_req_to_domain(payload, authorized_state.tenant_id),
            Some(authorized_state.actor_id),
        )
        .await
        .map(|x| (StatusCode::CREATED, Json(domain_to_rest(x))))
        .map_err(|e| {
            log::error!("Error handling insert_product_family: {}", e);
            RestApiError::from(e)
        })
}

/// Get product family
///
/// Retrieve a single product family by ID or alias.
#[utoipa::path(
    get,
    tag = "Product Families",
    path = "/api/v1/product_families/{id_or_alias}",
    params(
        ("id_or_alias" = String, Path, description = "product_family ID or alias")
    ),
    responses(
        (status = 200, description = "ProductFamily", body = ProductFamily),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error"),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn get_product_family_by_id_or_alias(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(id_or_alias): Path<AliasOr<ProductFamilyId>>,
) -> Result<impl IntoResponse, RestApiError> {
    // todo introduce alias
    let id = match id_or_alias {
        AliasOr::Id(id) => Ok(id),
        AliasOr::Alias(_) => Err(RestApiError::InvalidInput("alias not supported".to_string())),
    }?;

    app_state
        .store
        .find_product_family_by_id(id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error handling get_customer_by_id_or_alias: {}", e);
            RestApiError::from(e)
        })
        .map(domain_to_rest)
        .map(Json)
}
