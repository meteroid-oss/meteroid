use super::AppState;

use axum::extract::Query;
use axum::{Json, extract::State, response::IntoResponse};

use crate::api_rest::model::{PaginatedRequest, PaginatedResponse};
use crate::api_rest::plans::mapping::plan_to_rest;
use crate::api_rest::plans::model::{Plan, PlanListRequest};
use crate::errors::RestApiError;
use axum::Extension;
use axum::extract::Path;
use axum_valid::Valid;
use common_domain::ids::{PlanId, ProductFamilyId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::Store;
use meteroid_store::domain::{OrderByRequest, PlanVersionFilter};
use meteroid_store::repositories::PlansInterface;

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
        ("bearer_auth" = [])
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
    )
    .await
    .map(Json)
    .map_err(|e| {
        log::error!("Error handling list_plans: {e}");
        e
    })
}

async fn list_plans_handler(
    store: Store,
    pagination: PaginatedRequest,
    tenant_id: TenantId,
    product_family_id: Option<ProductFamilyId>,
) -> Result<PaginatedResponse<Plan>, RestApiError> {
    let res = store
        .list_full_plans(
            tenant_id,
            product_family_id,
            pagination.into(),
            OrderByRequest::IdAsc,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_plans: {e}");
            RestApiError::StoreError
        })?;

    let rest_models: Vec<Plan> = res
        .items
        .into_iter()
        .map(|full_plan| {
            plan_to_rest(
                full_plan.plan,
                full_plan.version,
                full_plan.price_components,
                full_plan.product_family.name,
            )
        })
        .collect::<Vec<_>>();

    Ok(PaginatedResponse {
        data: rest_models,
        total: res.total_results,
    })
}

#[utoipa::path(
    get,
    tag = "plan",
    path = "/api/v1/plans/{plan_id}",
    params(
        ("plan_id" = String, Path, description = "Plan ID"),
    ),
    responses(
        (status = 200, description = "Plan details", body = Plan),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Plan not found"),
        (status = 500, description = "Internal error"),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn get_plan_details(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(plan_id): Path<PlanId>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    get_plan_details_handler(app_state.store, authorized_state.tenant_id, plan_id)
        .await
        .map(Json)
        .map_err(|e| {
            log::error!("Error handling get_plan_details: {}", e);
            e
        })
}

async fn get_plan_details_handler(
    store: Store,
    tenant_id: TenantId,
    plan_id: PlanId,
) -> Result<Plan, RestApiError> {
    let full_plan = store
        .get_full_plan(plan_id, tenant_id, PlanVersionFilter::Active)
        .await
        .map_err(|e| {
            log::error!("Error fetching plan details: {}", e);
            RestApiError::StoreError
        })?;

    Ok(plan_to_rest(
        full_plan.plan,
        full_plan.version,
        full_plan.price_components,
        full_plan.product_family.name,
    ))
}
