use crate::AppState;
use crate::credits::router::resolve_customer;
use crate::resources::error::ApiError;
use crate::resources::model::{
    Resource, ResourceListRequest, ResourceListResponse, ResourceType, ResourceTypeCreateRequest,
    ResourceTypeListRequest, ResourceTypeListResponse, ResourceTypeUpdateRequest,
    ResourceUpsertRequest,
};
use axum::extract::State;
use axum::{Extension, Json};
use common_domain::ids::{AliasOr, CustomerId, ResourceTypeId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_rest_common::QueryParams;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_rest_common::extract::{ApiJson, ApiPath, ApiQuery, ApiValid};
use meteroid_rest_common::model::PaginationExt;
use meteroid_store::repositories::ResourcesInterface;
use meteroid_store::repositories::resources::{ResourceTypeNew, ResourceTypePatch, ResourceUpsert};

type ResourcePath = (AliasOr<CustomerId>, AliasOr<ResourceTypeId>, String);

/// List resource types
///
/// The tenant's resource types, by code.
#[utoipa::path(
    get,
    tag = "Resources",
    path = "/api/v1/resource-types",
    params(ResourceTypeListRequest),
    responses(
        (status = 200, description = "The tenant's resource types", body = ResourceTypeListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_resource_types(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    ApiValid(ApiQuery(request)): ApiValid<ApiQuery<ResourceTypeListRequest>>,
) -> Result<Json<ResourceTypeListResponse>, ApiError> {
    let types = state
        .store
        .list_resource_types(auth.tenant_id, request.include_archived.unwrap_or(false))
        .await?;
    Ok(Json(ResourceTypeListResponse {
        data: types.into_iter().map(Into::into).collect(),
    }))
}

/// Get a resource type
#[utoipa::path(
    get,
    tag = "Resources",
    path = "/api/v1/resource-types/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Resource type ID or code")),
    responses(
        (status = 200, description = "The resource type", body = ResourceType),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_resource_type(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    ApiPath(id_or_code): ApiPath<AliasOr<ResourceTypeId>>,
) -> Result<Json<ResourceType>, ApiError> {
    let resource_type = state
        .store
        .get_resource_type(auth.tenant_id, id_or_code)
        .await?;
    Ok(Json(resource_type.into()))
}

/// Create a resource type
///
/// A kind of thing a customer has several of, identified by its keys in checks and events.
#[utoipa::path(
    post,
    tag = "Resources",
    path = "/api/v1/resource-types",
    request_body(content = ResourceTypeCreateRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Created", body = ResourceType),
        (status = 400, description = "Invalid request, or `RESOURCE_HIERARCHY_UNSUPPORTED`", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 409, description = "The code is taken, or `RESOURCE_KEY_CLASH`", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_resource_type(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<ResourceTypeCreateRequest>>,
) -> Result<(StatusCode, Json<ResourceType>), ApiError> {
    let created = state
        .store
        .create_resource_type(ResourceTypeNew {
            tenant_id: auth.tenant_id,
            code: payload.code,
            name: payload.name,
            plural_name: payload.plural_name,
            parent_type_id: payload.parent_type_id,
            keys: payload.keys,
            attributes: payload.attributes,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(created.into())))
}

/// Update a resource type
///
/// Partial update. The code is immutable; archiving keeps the type resolving in checks.
#[utoipa::path(
    patch,
    tag = "Resources",
    path = "/api/v1/resource-types/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Resource type ID or code")),
    request_body(content = ResourceTypeUpdateRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Updated", body = ResourceType),
        (status = 400, description = "Invalid request, or `RESOURCE_HIERARCHY_UNSUPPORTED`", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
        (status = 409, description = "`RESOURCE_KEY_CLASH`, or removed attribute values still in use", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_resource_type(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    ApiPath(id_or_code): ApiPath<AliasOr<ResourceTypeId>>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<ResourceTypeUpdateRequest>>,
) -> Result<Json<ResourceType>, ApiError> {
    let updated = state
        .store
        .update_resource_type(
            auth.tenant_id,
            id_or_code,
            ResourceTypePatch {
                name: payload.name,
                plural_name: payload.plural_name,
                parent_type_id: payload.parent_type_id,
                keys: payload.keys,
                attributes: payload.attributes,
                archived: payload.archived,
            },
        )
        .await?;
    Ok(Json(updated.into()))
}

/// List a customer's resources
#[utoipa::path(
    get,
    tag = "Resources",
    path = "/api/v1/customers/{id_or_alias}/resources",
    params(("id_or_alias" = String, Path, description = "Customer ID or alias"), ResourceListRequest),
    responses(
        (status = 200, description = "The customer's stored resources", body = ResourceListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer or type not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_resources(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    ApiPath(id_or_alias): ApiPath<AliasOr<CustomerId>>,
    ApiValid(QueryParams(request)): ApiValid<QueryParams<ResourceListRequest>>,
) -> Result<Json<ResourceListResponse>, ApiError> {
    let customer = resolve_customer(&state, &auth, id_or_alias).await?;
    let page = state
        .store
        .list_resources(
            auth.tenant_id,
            Some(customer.id),
            request.resource_type,
            request.include_archived.unwrap_or(false),
            request.pagination.into(),
        )
        .await?;
    Ok(Json(ResourceListResponse {
        data: page.items.into_iter().map(Into::into).collect(),
        pagination_meta: request
            .pagination
            .into_response(page.total_pages, page.total_results),
    }))
}

/// Get a resource
#[utoipa::path(
    get,
    tag = "Resources",
    path = "/api/v1/customers/{id_or_alias}/resources/{type}/{key_value}",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
        ("type" = String, Path, description = "Resource type code or ID"),
        ("key_value" = String, Path, description = "The resource's key value, percent-encoded"),
    ),
    responses(
        (status = 200, description = "The resource", body = Resource),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_resource(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    ApiPath((id_or_alias, resource_type, key_value)): ApiPath<ResourcePath>,
) -> Result<Json<Resource>, ApiError> {
    let customer = resolve_customer(&state, &auth, id_or_alias).await?;
    let resource = state
        .store
        .find_resource(auth.tenant_id, customer.id, resource_type, &key_value)
        .await?;
    Ok(Json(resource.into()))
}

/// Upsert a resource
///
/// Creates the customer's resource with this type and key value, or replaces its name,
/// attributes and archive. Resources are never created by a check or an event.
#[utoipa::path(
    put,
    tag = "Resources",
    path = "/api/v1/customers/{id_or_alias}/resources/{type}/{key_value}",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
        ("type" = String, Path, description = "Resource type code or ID"),
        ("key_value" = String, Path, description = "The resource's key value, percent-encoded"),
    ),
    request_body(content = ResourceUpsertRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "The resource", body = Resource),
        (status = 400, description = "Invalid request, `RESOURCE_TYPE_ARCHIVED` or `RESOURCE_LIMIT_REACHED`", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer or type not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn upsert_resource(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    ApiPath((id_or_alias, resource_type, key_value)): ApiPath<ResourcePath>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<ResourceUpsertRequest>>,
) -> Result<Json<Resource>, ApiError> {
    let customer = resolve_customer(&state, &auth, id_or_alias).await?;
    let resource = state
        .store
        .upsert_resource(ResourceUpsert {
            tenant_id: auth.tenant_id,
            customer_id: customer.id,
            resource_type,
            key_value,
            name: payload.name,
            attributes: payload.attributes,
            archived: payload.archived,
        })
        .await?;
    Ok(Json(resource.into()))
}

/// Archive a resource
///
/// It keeps resolving in checks; upsert it with `archived: false` to restore it.
#[utoipa::path(
    post,
    tag = "Resources",
    path = "/api/v1/customers/{id_or_alias}/resources/{type}/{key_value}/archive",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
        ("type" = String, Path, description = "Resource type code or ID"),
        ("key_value" = String, Path, description = "The resource's key value, percent-encoded"),
    ),
    responses(
        (status = 200, description = "The archived resource", body = Resource),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_resource(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    ApiPath((id_or_alias, resource_type, key_value)): ApiPath<ResourcePath>,
) -> Result<Json<Resource>, ApiError> {
    let customer = resolve_customer(&state, &auth, id_or_alias).await?;
    let resource = state
        .store
        .find_resource(auth.tenant_id, customer.id, resource_type, &key_value)
        .await?;
    let archived = state
        .store
        .set_resource_archived(auth.tenant_id, resource.id, true)
        .await?;
    Ok(Json(archived.into()))
}

/// Delete a resource
///
/// Refused while a subscription (not aborted), quote or checkout session is for it, or once an
/// entitlement override, current or past, names it: archive it instead.
#[utoipa::path(
    delete,
    tag = "Resources",
    path = "/api/v1/customers/{id_or_alias}/resources/{type}/{key_value}",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
        ("type" = String, Path, description = "Resource type code or ID"),
        ("key_value" = String, Path, description = "The resource's key value, percent-encoded"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
        (status = 409, description = "A subscription, quote, checkout session or override names it", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn delete_resource(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    ApiPath((id_or_alias, resource_type, key_value)): ApiPath<ResourcePath>,
) -> Result<StatusCode, ApiError> {
    let customer = resolve_customer(&state, &auth, id_or_alias).await?;
    let resource = state
        .store
        .find_resource(auth.tenant_id, customer.id, resource_type, &key_value)
        .await?;
    state
        .store
        .delete_resource(auth.tenant_id, resource.id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
