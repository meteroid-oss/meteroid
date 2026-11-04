use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::CustomPropertyDefinitionId;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_app::errors::RestApiError;
use meteroid_rest_common::customproperties::mapping::{domain_to_rest, entity_type_to_domain};
use meteroid_rest_common::customproperties::model::{
    CustomPropertyDefinition, CustomPropertyDefinitionListRequest,
    CustomPropertyDefinitionListResponse,
};
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_rest_common::model::PaginationExt;
use meteroid_store::repositories::CustomPropertyDefinitionsInterface;

/// List custom property definitions
#[utoipa::path(
    get,
    tag = "Custom Properties",
    path = "/api/v1/custom-property-definitions",
    params(CustomPropertyDefinitionListRequest),
    responses(
        (status = 200, description = "List of custom property definitions", body = CustomPropertyDefinitionListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn list_definitions(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(Query(request)): Valid<Query<CustomPropertyDefinitionListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let entity_type = request.entity_type.map(entity_type_to_domain);
    let defs = app_state
        .store
        .list_custom_property_definitions(
            authorized_state.tenant_id,
            entity_type,
            request.include_archived.unwrap_or(false),
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_custom_property_definitions: {e}");
            RestApiError::from(e)
        })?;

    let total = defs.len() as u64;
    let data: Vec<CustomPropertyDefinition> = defs.into_iter().map(domain_to_rest).collect();

    Ok(Json(CustomPropertyDefinitionListResponse {
        data,
        pagination_meta: request.pagination.into_response(1, total),
    }))
}

/// Get a custom property definition
#[utoipa::path(
    get,
    tag = "Custom Properties",
    path = "/api/v1/custom-property-definitions/{id}",
    params(("id" = String, Path, description = "custom property definition ID")),
    responses(
        (status = 200, description = "Custom property definition", body = CustomPropertyDefinition),
        (status = 404, description = "Not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn get_definition(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(id): Path<CustomPropertyDefinitionId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .get_custom_property_definition(authorized_state.tenant_id, id)
        .await
        .map_err(RestApiError::from)
        .map(|d| Json(domain_to_rest(d)))
}
