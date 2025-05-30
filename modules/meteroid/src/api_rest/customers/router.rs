use crate::api_rest::AppState;
use crate::api_rest::customers::mapping::{
    create_req_to_domain, domain_to_rest, update_req_to_domain,
};
use crate::api_rest::customers::model::{
    Customer, CustomerCreateRequest, CustomerListRequest, CustomerUpdateRequest,
};
use crate::api_rest::model::PaginatedResponse;
use crate::errors::RestApiError;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{AliasOr, CustomerId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_store::domain::OrderByRequest;
use meteroid_store::repositories::CustomersInterface;

#[utoipa::path(
    get,
    tag = "customer",
    path = "/api/v1/customers",
    params(
        ("offset" = usize, Query, description = "Specifies the starting position of the results", example = 0, minimum = 0),
        ("limit" = usize, Query, description = "The maximum number of objects to return", example = 10, minimum = 1),
        ("search" = String, Query, description = "Filtering criteria", example = "abc"),
    ),
    responses(
        (status = 200, description = "List of customers", body = PaginatedResponse<Customer>),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error"),
    ),
    security(
        ("api-key" = [])
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
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_customers: {}", e);
            RestApiError::from(e)
        })?;

    let items = res
        .items
        .into_iter()
        .map(domain_to_rest)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(PaginatedResponse {
        data: items,
        total: res.total_results,
    }))
}

#[utoipa::path(
    get,
    tag = "customer",
    path = "/api/v1/customers/{id_or_alias}",
    params(
        ("id_or_alias" = String, Path, description = "customer ID or alias")
    ),
    responses(
        (status = 200, description = "Customer", body = Customer),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error"),
    ),
    security(
        ("api-key" = [])
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
            log::error!("Error handling get_customer: {}", e);
            RestApiError::from(e)
        })
        .and_then(domain_to_rest)
        .map(Json)
}

#[utoipa::path(
    post,
    tag = "customer",
    path = "/api/v1/customers",
    request_body(content = CustomerCreateRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Customer successfully created", body = Customer),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Customer not found"),
        (status = 409, description = "Customer already exists"),
        (status = 500, description = "Internal error"),
    ),
    security(
        ("api-key" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn create_customer(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<CustomerCreateRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let created = app_state
        .store
        .insert_customer(
            create_req_to_domain(authorized_state.actor_id, payload),
            authorized_state.tenant_id,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling insert_customer: {}", e);
            RestApiError::from(e)
        })?;

    app_state
        .store
        .find_customer_by_id_or_alias(AliasOr::Id(created.id), authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error handling get_customer: {}", e);
            RestApiError::from(e)
        })
        .and_then(domain_to_rest)
        .map(|x| (StatusCode::CREATED, Json(x)))
}

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
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Customer not found"),
        (status = 500, description = "Internal error"),
    ),
    security(
        ("api-key" = [])
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
            log::error!("Error handling update_customer: {}", e);
            RestApiError::from(e)
        })
        .and_then(domain_to_rest)
        .map(Json)
}

#[utoipa::path(
    delete,
    tag = "customer",
    path = "/api/v1/customers/{id_or_alias}",
    params(
        ("id_or_alias" = String, Path, description = "customer ID or alias")
    ),
    responses(
        (status = 200, description = "Customer"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Customer not found"),
        (status = 500, description = "Internal error"),
    ),
    security(
        ("api-key" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn delete_customer(
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
            log::error!("Error handling delete_customer: {}", e);
            RestApiError::from(e)
        })
        .map(Json)
}
