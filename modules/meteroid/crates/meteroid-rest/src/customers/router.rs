use crate::AppState;
use crate::auth::DelegatedCredentials;
use crate::customers::mapping::{
    create_req_to_domain, domain_to_rest, patch_req_to_domain, update_req_to_domain,
};
use crate::customers::model::{
    Customer, CustomerCreateRequest, CustomerListRequest, CustomerListResponse,
    CustomerPatchRequest, CustomerPortalTokenRequest, CustomerPortalTokenResponse,
    CustomerUpdateRequest,
};
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{AliasOr, BaseId, CustomerId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_app::errors::RestApiError;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_rest_common::model::{PaginationExt, validate_order_by};
use meteroid_store::jwt_claims::{
    PortalJwtClaims, ResourceAccess, default_customer_portal_token_ttl,
};
use meteroid_store::repositories::CustomersInterface;

/// List customers
///
/// List customers with optional pagination and search filtering.
#[utoipa::path(
    get,
    tag = "Customers",
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
pub async fn list_customers(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(Query(request)): Valid<Query<CustomerListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let order_by = validate_order_by(
        &request.order_by,
        &["name", "email", "alias", "created_at"],
        "created_at.desc",
    )
    .map_err(RestApiError::InvalidInput)?;

    let res = app_state
        .store
        .list_customers(
            authorized_state.tenant_id,
            request.pagination.into(),
            Some(order_by),
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
    tag = "Customers",
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
pub async fn get_customer(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .find_customer_by_id_or_alias(id_or_alias.clone(), authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::warn!(
                "get_customer: failed to find customer '{id_or_alias}' for tenant {}: {e}",
                authorized_state.tenant_id.as_uuid()
            );
            RestApiError::from(e)
        })
        .and_then(domain_to_rest)
        .map(Json)
}

/// Create customer
#[utoipa::path(
    post,
    tag = "Customers",
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
pub async fn create_customer(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<CustomerCreateRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let created = app_state
        .store
        .insert_customer(
            authorized_state.as_actor(),
            create_req_to_domain(payload)?,
            authorized_state.tenant_id,
        )
        .await
        .map_err(|e| {
            log::error!("create_customer: failed to insert customer: {e}");
            RestApiError::from(e)
        })?;

    app_state
        .store
        .find_customer_by_id_or_alias(AliasOr::Id(created.id), authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!(
                "create_customer: failed to fetch created customer {}: {e}",
                created.id
            );
            RestApiError::from(e)
        })
        .and_then(domain_to_rest)
        .map(|x| (StatusCode::CREATED, Json(x)))
}

/// Update customer
#[utoipa::path(
    put,
    tag = "Customers",
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
pub async fn update_customer(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
    Valid(Json(payload)): Valid<Json<CustomerUpdateRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    // Loaded so the mapping can preserve stored B2C identity that the payload omits.
    let existing = app_state
        .store
        .find_customer_by_id_or_alias(id_or_alias.clone(), authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::warn!("update_customer: failed to find customer '{id_or_alias}': {e}");
            RestApiError::from(e)
        })?;

    app_state
        .store
        .update_customer(
            authorized_state.as_actor(),
            authorized_state.tenant_id,
            update_req_to_domain(id_or_alias.clone(), payload, &existing)?,
        )
        .await
        .map_err(|e| {
            log::error!("update_customer: failed to update customer '{id_or_alias}': {e}");
            RestApiError::from(e)
        })
        .and_then(domain_to_rest)
        .map(Json)
}

/// Patch customer
///
/// Partially update a customer. Only provided fields will be updated.
#[utoipa::path(
    patch,
    tag = "Customers",
    path = "/api/v1/customers/{id_or_alias}",
    params(
        ("id_or_alias" = String, Path, description = "customer ID or alias")
    ),
    request_body(content = CustomerPatchRequest, content_type = "application/json"),
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
pub async fn patch_customer(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
    Valid(Json(payload)): Valid<Json<CustomerPatchRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    // First resolve the customer ID
    let customer = app_state
        .store
        .find_customer_by_id_or_alias(id_or_alias.clone(), authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::warn!("patch_customer: failed to find customer '{id_or_alias}': {e}");
            RestApiError::from(e)
        })?;

    let updated = app_state
        .store
        .patch_customer(
            authorized_state.as_actor(),
            authorized_state.tenant_id,
            patch_req_to_domain(customer.id, payload)?,
        )
        .await
        .map_err(|e| {
            log::error!(
                "patch_customer: failed to patch customer {}: {e}",
                customer.id
            );
            RestApiError::from(e)
        })?;

    match updated {
        Some(customer) => domain_to_rest(customer).map(Json),
        None => Err(RestApiError::NotFound),
    }
}

/// Archive a customer
///
/// No linked entity will be deleted. You need to terminate all active subscriptions before archiving a customer, or the call will fail.
#[utoipa::path(
    delete,
    tag = "Customers",
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
pub async fn archive_customer(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .archive_customer(
            authorized_state.as_actor(),
            authorized_state.tenant_id,
            id_or_alias.clone(),
        )
        .await
        .map_err(|e| {
            log::error!("archive_customer: failed to archive customer '{id_or_alias}': {e}");
            RestApiError::from(e)
        })
        .map(|()| StatusCode::NO_CONTENT)
}

/// Restore an archived customer
#[utoipa::path(
    post,
    tag = "Customers",
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
pub async fn unarchive_customer(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .unarchive_customer(
            authorized_state.as_actor(),
            authorized_state.tenant_id,
            id_or_alias.clone(),
        )
        .await
        .map_err(|e| {
            log::error!("unarchive_customer: failed to unarchive customer '{id_or_alias}': {e}");
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
    tag = "Customers",
    path = "/api/v1/customers/{id_or_alias}/portal-token",
    params(
        ("id_or_alias" = String, Path, description = "customer ID or alias")
    ),
    // Declared as required so the body maps to a plain parameter in the generated clients; the
    // handler still accepts a missing body, and every field inside the request is optional.
    request_body(content = CustomerPortalTokenRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Portal token generated", body = CustomerPortalTokenResponse),
        (status = 400, description = "Invalid input", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub async fn create_portal_token(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    delegated: Option<Extension<DelegatedCredentials>>,
    State(app_state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
    body: Option<Json<CustomerPortalTokenRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    // The request body is optional; default to a 24h token with every scope when omitted.
    let request = body.map(|Json(b)| b).unwrap_or_default();
    let expires_in = match request.expires_in_seconds {
        Some(seconds) => {
            if !(60..=2_592_000).contains(&seconds) {
                return Err(RestApiError::InvalidInput(
                    "expires_in_seconds must be between 60 and 2592000 (30 days)".into(),
                ));
            }
            chrono::Duration::seconds(seconds as i64)
        }
        None => default_customer_portal_token_ttl(),
    };
    if request.scopes.as_ref().is_some_and(Vec::is_empty) {
        return Err(RestApiError::InvalidInput(
            "scopes must not be empty".into(),
        ));
    }
    let scopes = request
        .scopes
        .map(|scopes| scopes.into_iter().map(Into::into).collect());

    // Verify customer exists and belongs to tenant
    let customer = app_state
        .store
        .find_customer_by_id_or_alias(id_or_alias.clone(), authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::warn!("create_portal_token: failed to find customer '{id_or_alias}': {e}");
            RestApiError::from(e)
        })?;

    let claims = PortalJwtClaims::new(
        authorized_state.tenant_id,
        ResourceAccess::Customer(customer.id),
        scopes,
        Some(expires_in),
    )
    .map_err(RestApiError::from)?
    .with_delegated(delegated.is_some());
    let token = claims.encode(&app_state.jwt_secret).map_err(|e| {
        log::error!("Error generating portal token: {e}");
        RestApiError::StoreError
    })?;
    let expires_at = claims.expires_at().ok_or(RestApiError::StoreError)?;

    let portal_url = app_state.portal_url.clone();
    let portal_link = format!(
        "{}/portal/customer?token={token}",
        portal_url.trim_end_matches('/')
    );

    Ok(Json(CustomerPortalTokenResponse {
        token,
        expires_at: expires_at.into(),
        portal_url,
        portal_link,
        api_url: app_state.api_url.clone(),
    }))
}
