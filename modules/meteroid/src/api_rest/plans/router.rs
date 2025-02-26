use super::AppState;

use axum::extract::Query;
use axum::{extract::State, response::IntoResponse, Json};

use crate::api_rest::model::{PaginatedRequest, PaginatedResponse};
use crate::api_rest::plans::mapping::domain_to_rest;
use crate::api_rest::plans::model::{Plan, PlanFilters, PlanListRequest};
use crate::errors::RestApiError;
use axum::Extension;
use axum_valid::Valid;
use common_domain::ids::{ProductFamilyId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::domain::OrderByRequest;
use meteroid_store::repositories::PlansInterface;
use meteroid_store::{domain, Store};

#[utoipa::path(
    get,
    tag = "plan",
    path = "/api/v1/plans",
    params(
        ("offset" = usize, Query, description = "Specifies the starting position of the results", example = 0, minimum = 0),
        ("limit" = usize, Query, description = "The maximum number of objects to return", example = 10, minimum = 1),
        ("product_family_id" = String, Query, description = "Product family ID", example = "default"),
        ("search" = String, Query, description = "Filtering criteria", example = "abc"),
    ),
    responses(
        (status = 200, description = "List of plans", body = PaginatedResponse<Plan>),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error"),
    ),
    security(
        ("api-key" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn list_plans(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(Query(request)): Valid<Query<PlanListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    list_plans_handler(
        app_state.store,
        request.pagination,
        authorized_state.tenant_id,
        request.product_family_id,
        request.plan_filters,
    )
    .await
    .map(Json)
    .map_err(|e| {
        log::error!("Error handling list_plans: {}", e);
        e
    })
}

async fn list_plans_handler(
    store: Store,
    pagination: PaginatedRequest,
    tenant_id: TenantId,
    product_family_id: Option<ProductFamilyId>,
    plan_filters: PlanFilters,
) -> Result<PaginatedResponse<Plan>, RestApiError> {
    let res = store
        .list_plans(
            tenant_id,
            product_family_id,
            domain::PlanFilters {
                search: plan_filters.search,
                filter_status: Vec::new(),
                filter_type: Vec::new(),
            },
            domain::PaginationRequest {
                page: pagination.offset.unwrap_or(0),
                per_page: pagination.limit,
            },
            OrderByRequest::IdAsc,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_plans: {}", e);
            RestApiError::StoreError
        })?;

    let rest_models: Vec<Plan> = res
        .items
        .into_iter()
        .map(domain_to_rest)
        .collect::<Vec<_>>();

    Ok(PaginatedResponse {
        data: rest_models,
        total: res.total_results,
        offset: res.total_pages,
    })
}
