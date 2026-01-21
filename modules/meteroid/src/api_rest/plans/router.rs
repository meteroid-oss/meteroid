use super::AppState;

use axum::extract::Query;
use axum::{Json, extract::State, response::IntoResponse};

use crate::api_rest::model::{PaginatedRequest, PaginationExt};
use crate::api_rest::plans::mapping::plan_to_rest;
use crate::api_rest::plans::model::{Plan, PlanListRequest, PlanListResponse};
use crate::errors::RestApiError;
use axum::Extension;
use axum::extract::Path;
use axum_valid::Valid;
use common_domain::ids::{PlanId, ProductFamilyId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::Store;
use meteroid_store::domain::{OrderByRequest, PlanVersionFilter};
use meteroid_store::repositories::PlansInterface;

/// List plans
///
/// List plans with optional filtering by product family.
#[utoipa::path(
    get,
    tag = "Plans",
    path = "/api/v1/plans",
    params(
        PlanListRequest
    ),
    responses(
        (status = 200, description = "List of plans", body = PlanListResponse),
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
) -> Result<PlanListResponse, RestApiError> {
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

    Ok(PlanListResponse {
        data: rest_models,
        pagination_meta: pagination.into_response(res.total_pages, res.total_results),
    })
}

/// Get plan
///
/// Retrieve the details of a specific plan
#[utoipa::path(
    get,
    tag = "Plans",
    path = "/api/v1/plans/{plan_id}",
    params(
        ("plan_id" = PlanId, Path, description = "Plan ID"),
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
