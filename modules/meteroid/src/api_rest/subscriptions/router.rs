use super::AppState;

use axum::extract::Query;
use axum::{extract::State, response::IntoResponse, Json};

use axum::Extension;

use crate::api_rest::model::{PaginatedRequest, PaginatedResponse};
use crate::api_rest::subscriptions::mapping::domain_to_rest;
use crate::api_rest::subscriptions::model::{Subscription, SubscriptionRequest};
use crate::errors::RestApiError;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::{domain, Store};
use utoipa::OpenApi;
use uuid::Uuid;

#[derive(OpenApi)]
#[openapi(paths(list_subscriptions))]
pub struct SubscriptionApi;

#[utoipa::path(
    get,
    tag = "subscription",
    path = "/v1/subscriptions",
    params(
        ("offset" = usize, Query, description = "Specifies the starting position of the results", example = 1, minimum = 0),
        ("limit" = usize, Query, description = "The maximum number of objects to return", example = 10, minimum = 1)
    ),
    responses(
        (status = 200, description = "List of subscriptions", body = PaginatedResponse<Subscription>),
        (status = 500, description = "Internal error"),
    )
)]
#[axum::debug_handler]
pub async fn list_subscriptions(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Query(request): Query<SubscriptionRequest>,
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
    tenant_id: Uuid,
    customer_id: Option<Uuid>,
    plan_id: Option<Uuid>,
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
        .iter()
        .map(|v| domain_to_rest(v.clone()))
        .collect::<Vec<_>>();

    Ok(PaginatedResponse {
        data: subscriptions,
        total: res.total_results,
        offset: res.total_pages,
    })
}
