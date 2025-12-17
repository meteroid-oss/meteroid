use crate::api_rest::AppState;
use crate::api_rest::customers::mapping::{
    create_req_to_domain, domain_to_rest, update_req_to_domain,
};
use crate::api_rest::customers::model::{
    Customer, CustomerCreateRequest, CustomerListRequest, CustomerListResponse,
    CustomerPortalTokenResponse, CustomerUpdateRequest,
};
use crate::api_rest::error::RestErrorResponse;
use crate::api_rest::model::PaginationExt;
use crate::errors::RestApiError;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{AliasOr, CustomerId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_store::domain::OrderByRequest;
use meteroid_store::jwt_claims::{ResourceAccess, generate_portal_token};
use meteroid_store::repositories::CustomersInterface;

/// List customers
///
/// List customers with optional pagination and search filtering.
#[utoipa::path(
    get,
    tag = "customer",
    path = "/api/v1/customers",
    params(
        CustomerListRequest
    ),
    responses(
        (status = 200, description = "List of customers", body = CustomerListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn list_customers(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(Query(request)): Valid<Query<CustomerListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .list_customers(
            authorized_state.tenant_id,
            request.pagination.into(),
            OrderByRequest::IdAsc,
            request.customer_filters.search,
            request.customer_filters.archived,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_customers: {e}");
            RestApiError::from(e)
        })?;

    let items = res
        .items
        .into_iter()
        .map(domain_to_rest)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(CustomerListResponse {
        data: items,
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

/// Get customer
///
/// Retrieve a single customer by ID or alias.
#[utoipa::path(
    get,
    tag = "customer",
    path = "/api/v1/customers/{id_or_alias}",
    params(
        ("id_or_alias" = String, Path, description = "customer ID or alias")
    ),
    responses(
        (status = 200, description = "Customer", body = Customer),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn get_customer(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .find_customer_by_id_or_alias(id_or_alias, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error handling get_customer: {e}");
            RestApiError::from(e)
        })
        .and_then(domain_to_rest)
        .map(Json)
}

/// Create customer
#[utoipa::path(
    post,
    tag = "customer",
    path = "/api/v1/customers",
    request_body(content = CustomerCreateRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Customer successfully created", body = Customer),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 409, description = "Customer already exists", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn create_customer(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<CustomerCreateRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    log::info!("Creating customer with payload: {:?}", payload);

    let created = app_state
        .store
        .insert_customer(
            create_req_to_domain(authorized_state.actor_id, payload),
            authorized_state.tenant_id,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling insert_customer: {e}");
            RestApiError::from(e)
        })?;

    app_state
        .store
        .find_customer_by_id_or_alias(AliasOr::Id(created.id), authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error handling get_customer: {e}");
            RestApiError::from(e)
        })
        .and_then(domain_to_rest)
        .map(|x| (StatusCode::CREATED, Json(x)))
}

/// Update customer
#[utoipa::path(
    put,
    tag = "customer",
    path = "/api/v1/customers/{id_or_alias}",
    params(
        ("id_or_alias" = String, Path, description = "customer ID or alias")
    ),
    request_body(content = CustomerUpdateRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Customer", body = Customer),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn update_customer(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
    Valid(Json(payload)): Valid<Json<CustomerUpdateRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .update_customer(
            authorized_state.actor_id,
            authorized_state.tenant_id,
            update_req_to_domain(id_or_alias, payload),
        )
        .await
        .map_err(|e| {
            log::error!("Error handling update_customer: {e}");
            RestApiError::from(e)
        })
        .and_then(domain_to_rest)
        .map(Json)
}

/// Archive a customer
///
/// No linked entity will be deleted. You need to terminate all active subscriptions before archiving a customer, or the call will fail.
#[utoipa::path(
    delete,
    tag = "customer",
    path = "/api/v1/customers/{id_or_alias}",
    params(
        ("id_or_alias" = String, Path, description = "customer ID or alias")
    ),
    responses(
        (status = 204, description = "No Content"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn archive_customer(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .archive_customer(
            authorized_state.actor_id,
            authorized_state.tenant_id,
            id_or_alias,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling archive_customer: {e}");
            RestApiError::from(e)
        })
        .map(|()| StatusCode::NO_CONTENT)
}

/// Restore an archived customer
#[utoipa::path(
    post,
    tag = "customer",
    path = "/api/v1/customers/{id_or_alias}/unarchive",
    params(
        ("id_or_alias" = String, Path, description = "customer ID or alias")
    ),
    responses(
        (status = 204, description = "No Content"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn unarchive_customer(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .unarchive_customer(authorized_state.tenant_id, id_or_alias)
        .await
        .map_err(|e| {
            log::error!("Error handling unarchive_customer: {e}");
            RestApiError::from(e)
        })
        .map(|()| StatusCode::NO_CONTENT)
}

/// Generate a portal token for a customer
///
/// Generates a JWT token that grants access to the customer portal.
/// The token can be used to access invoices, payment methods, and other portal features.
#[utoipa::path(
    post,
    tag = "customer",
    path = "/api/v1/customers/{id_or_alias}/portal-token",
    params(
        ("id_or_alias" = String, Path, description = "customer ID or alias")
    ),
    responses(
        (status = 200, description = "Portal token generated", body = CustomerPortalTokenResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn create_portal_token(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
) -> Result<impl IntoResponse, RestApiError> {
    // Verify customer exists and belongs to tenant
    let customer = app_state
        .store
        .find_customer_by_id_or_alias(id_or_alias, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error finding customer: {e}");
            RestApiError::from(e)
        })?;

    let token = generate_portal_token(
        &app_state.jwt_secret,
        authorized_state.tenant_id,
        ResourceAccess::Customer(customer.id),
    )
    .map_err(|e| {
        log::error!("Error generating portal token: {e}");
        RestApiError::StoreError
    })?;

    let portal_url = app_state.portal_url.clone();

    Ok(Json(CustomerPortalTokenResponse { token, portal_url }))
}
