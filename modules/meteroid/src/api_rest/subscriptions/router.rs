use super::AppState;

use axum::extract::Query;
use axum::routing::get;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};

use axum::{Extension, Router};
use hyper::StatusCode;

use crate::{api, errors};

use crate::api::sharable::ShareableEntityClaims;
use crate::api::subscriptions::error::SubscriptionApiError;
use crate::api_rest::extract_tenant;
use crate::api_rest::model::{PaginatedRequest, PaginatedResponse};
use crate::api_rest::subscriptions::mapping::domain_to_rest;
use crate::api_rest::subscriptions::model::{Subscription, SubscriptionRequest};
use crate::errors::RestApiError;
use crate::services::storage::Prefix;
use common_grpc::middleware::server::AuthorizedState;
use fang::Deserialize;
use image::ImageFormat::Png;
use jsonwebtoken::{decode, DecodingKey, Validation};
use meteroid_store::repositories::{InvoiceInterface, SubscriptionInterface};
use meteroid_store::{domain, Store};
use secrecy::ExposeSecret;
use utoipa::{IntoParams, OpenApi, ToSchema};
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
    Extension(authorized_state): Extension<AuthorizedState>,
    Query(request): Query<SubscriptionRequest>,
    State(app_state): State<AppState>,
) -> Response {
    let tenant_id = extract_tenant(authorized_state).unwrap();

    match list_subscriptions_handler(
        app_state.store,
        request.pagination,
        tenant_id,
        request.customer_id,
        request.plan_id,
    )
    .await
    {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            log::error!("Error handling logo: {}", e);
            // todo add mapping for RestApiError
            (StatusCode::INTERNAL_SERVER_ERROR, "error").into_response()
        }
    }
}

async fn list_subscriptions_handler(
    store: Store,
    pagination: Option<PaginatedRequest>,
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
                page: pagination.as_ref().map(|p| p.offset).unwrap_or(0),
                per_page: pagination.as_ref().map(|p| p.limit),
            },
        )
        .await
        .map_err(|e| RestApiError::StoreError)?;

    let subscriptions: Vec<Subscription> = res
        .items
        .into_iter()
        .map(domain_to_rest)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(PaginatedResponse {
        data: subscriptions,
        total: res.total_results,
        offset: res.total_pages,
    })
}
