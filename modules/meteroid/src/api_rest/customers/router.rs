use crate::api_rest::customers::mapping::{create_req_to_domain, domain_to_rest};
use crate::api_rest::customers::model::{Customer, CustomerCreateRequest, CustomerListRequest};
use crate::api_rest::model::PaginatedResponse;
use crate::api_rest::AppState;
use crate::errors::RestApiError;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_store::domain;
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
        (status = 500, description = "Internal error"),
    )
)]
#[axum::debug_handler]
pub(crate) async fn list_customers(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Query(request): Query<CustomerListRequest>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .list_customers_for_display(
            authorized_state.tenant_id,
            domain::PaginationRequest {
                page: request.pagination.offset.unwrap_or(0),
                per_page: request.pagination.limit,
            },
            OrderByRequest::IdAsc,
            request.plan_filters.search,
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
        offset: res.total_pages,
    }))
}

#[utoipa::path(
    get,
    tag = "customer",
    path = "/api/v1/customers/:id_or_alias",
    params(
        ("id_or_alias" = String, Path, description = "customer ID or alias")
    ),
    responses(
        (status = 200, description = "Customer", body = Customer),
        (status = 500, description = "Internal error"),
    )
)]
#[axum::debug_handler]
pub(crate) async fn get_customer_by_id_or_alias(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(id_or_alias): Path<String>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .find_customer_by_local_id_or_alias(id_or_alias, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error handling get_customer_by_id_or_alias: {}", e);
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
        (status = 404, description = "Customer Not Found"),
        (status = 409, description = "Customer already exists"),
        (status = 500, description = "Internal error"),
    )
)]
#[axum::debug_handler]
pub(crate) async fn create_customer(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Json(payload): Json<CustomerCreateRequest>,
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
        .find_customer_by_local_id_or_alias(created.local_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error handling get_customer_by_id_or_alias: {}", e);
            RestApiError::from(e)
        })
        .and_then(domain_to_rest)
        .map(|x| (StatusCode::CREATED, Json(x)))
}
