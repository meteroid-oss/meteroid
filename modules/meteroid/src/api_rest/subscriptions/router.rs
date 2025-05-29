use super::AppState;

use axum::extract::{Path, Query};
use axum::{Json, extract::State, response::IntoResponse};

use crate::api_rest::model::{PaginatedRequest, PaginatedResponse};
use crate::api_rest::subscriptions::mapping::{domain_to_rest, domain_to_rest_details};
use crate::api_rest::subscriptions::model::{
    Subscription, SubscriptionDetails, SubscriptionRequest,
};
use crate::errors::RestApiError;
use axum::Extension;
use axum_valid::Valid;
use common_domain::ids::{CustomerId, PlanId, SubscriptionId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use meteroid_store::{Store, domain};

#[utoipa::path(
    get,
    tag = "subscription",
    path = "/api/v1/subscriptions",
    params(
        ("offset" = usize, Query, description = "Specifies the starting position of the results", example = 0, minimum = 0),
        ("limit" = usize, Query, description = "The maximum number of objects to return", example = 10, minimum = 1)
    ),
    responses(
        (status = 200, description = "List of subscriptions", body = PaginatedResponse<Subscription>),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error"),
    ),
    security(
        ("api-key" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn list_subscriptions(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(Query(request)): Valid<Query<SubscriptionRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    list_subscriptions_handler(
        app_state.store,
        request.pagination,
        authorized_state.tenant_id,
        request.customer_id,
        request.plan_id,
    )
    .await
    .map(Json)
    .map_err(|e| {
        log::error!("Error handling list_subscriptions: {}", e);
        e
    })
}

async fn list_subscriptions_handler(
    store: Store,
    pagination: PaginatedRequest,
    tenant_id: TenantId,
    customer_id: Option<CustomerId>,
    plan_id: Option<PlanId>,
) -> Result<PaginatedResponse<Subscription>, RestApiError> {
    let res = store
        .list_subscriptions(
            tenant_id,
            customer_id,
            plan_id,
            domain::PaginationRequest {
                page: pagination.offset.unwrap_or(0),
                per_page: pagination.limit,
            },
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_subscriptions: {}", e);
            RestApiError::StoreError
        })?;

    let subscriptions: Vec<Subscription> = res
        .items
        .into_iter()
        .map(domain_to_rest)
        .collect::<Vec<_>>();

    Ok(PaginatedResponse {
        data: subscriptions,
        total: res.total_results,
        offset: res.total_pages,
    })
}

#[utoipa::path(
    get,
    tag = "subscription",
    path = "/api/v1/subscriptions/{id}",
    params(
        ("id" = String, Path, description = "subscription ID")
    ),
    responses(
        (status = 200, description = "Details of subscription", body = SubscriptionDetails),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error"),
    ),
    security(
        ("api-key" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn subscription_details(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(id): Path<SubscriptionId>,
) -> Result<impl IntoResponse, RestApiError> {
    subscription_details_handler(app_state.store, authorized_state.tenant_id, id)
        .await
        .map(Json)
        .map_err(|e| {
            log::error!("Error handling list_subscriptions: {}", e);
            e
        })
}

async fn subscription_details_handler(
    store: Store,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> Result<SubscriptionDetails, RestApiError> {
    let res = store
        .get_subscription_details(tenant_id, subscription_id)
        .await
        .map_err(|e| {
            log::error!("Error handling subscription_details: {}", e);
            RestApiError::StoreError
        })?;

    Ok(domain_to_rest_details(res))
}
