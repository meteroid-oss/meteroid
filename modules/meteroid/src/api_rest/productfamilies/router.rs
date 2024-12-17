use super::AppState;

use axum::{extract::State, response::IntoResponse, Json};

use crate::api_rest::model::PaginatedResponse;
use crate::api_rest::productfamilies::mapping::domain_to_rest;
use crate::api_rest::productfamilies::model::ProductFamily;
use crate::errors::RestApiError;
use axum::Extension;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::repositories::ProductFamilyInterface;
use meteroid_store::Store;
use rust_decimal::prelude::ToPrimitive;
use uuid::Uuid;

#[utoipa::path(
    get,
    tag = "product_family",
    path = "/api/v1/product_families",
    params(
    ),
    responses(
        (status = 200, description = "List of product families", body = PaginatedResponse<ProductFamily>),
        (status = 500, description = "Internal error"),
    )
)]
#[axum::debug_handler]
pub(crate) async fn list_product_families(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    list_product_families_handler(app_state.store, authorized_state.tenant_id)
        .await
        .map(Json)
        .map_err(|e| {
            log::error!("Error handling list_product_families: {}", e);
            e
        })
}

async fn list_product_families_handler(
    store: Store,
    tenant_id: Uuid,
) -> Result<PaginatedResponse<ProductFamily>, RestApiError> {
    let res = store.list_product_families(tenant_id).await.map_err(|e| {
        log::error!("Error handling list_product_families: {}", e);
        RestApiError::StoreError
    })?;

    let rest_models: Vec<ProductFamily> = res
        .iter()
        .map(|v| domain_to_rest(v.clone()))
        .collect::<Vec<_>>();

    Ok(PaginatedResponse {
        data: rest_models.clone(),
        total: rest_models.len().to_u64().unwrap(),
        offset: 0,
    })
}
