use crate::api_rest::customers::mapping::domain_to_rest;
use crate::api_rest::customers::model::{Customer, CustomerListRequest};
use crate::api_rest::model::PaginatedResponse;
use crate::api_rest::AppState;
use crate::errors::RestApiError;
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
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
        .list_customers(
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
            RestApiError::StoreError
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
