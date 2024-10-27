use super::{extract_tenant, AppState};
use std::io::Cursor;

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

use crate::api::axum_routers::model::{PaginatedRequest, PaginatedResponse};
use crate::api::axum_routers::subscription_router::rest_model::Subscription;
use crate::api::sharable::ShareableEntityClaims;
use crate::api::subscriptions::error::SubscriptionApiError;
use crate::errors::RestApiError;
use crate::services::storage::Prefix;
use common_grpc::middleware::server::AuthorizedState;
use error_stack::{Report, Result, ResultExt};
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

pub fn subscription_routes() -> Router<AppState> {
    Router::new().route("/v1/subscriptions", get(list_subscriptions))
}

#[utoipa::path(
    get,
    tag = "subscription",
    path = "/v1/subscriptions",
    params(
        ("offset" = usize, Query, description = "Specifies the starting position of the results", example = 1),
        ("limit" = usize, Query, description = "The maximum number of objects to return", example = 10)
    ),
    responses(
        (status = 200, content_type = "image/png", description = "Logo as PNG image", body = [u8]),
        (status = 400, description = "Invalid UUID"),
        (status = 500, description = "Internal error"),
    )
)]
#[axum::debug_handler]
async fn list_subscriptions(
    Extension(authorized_state): Extension<AuthorizedState>,
    Query(pagination): Query<Option<PaginatedRequest>>,
    Query(customer_id): Query<Option<Uuid>>,
    Query(plan_id): Query<Option<Uuid>>,
    State(app_state): State<AppState>,
) -> Response {
    let tenant_id = extract_tenant(authorized_state)?;

    match list_subscriptions_handler(app_state.store, pagination, tenant_id, customer_id, plan_id)
        .await
    {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            log::error!("Error handling logo: {}", e);
            e.current_context().clone().into_response()
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

    let subscriptions: Vec<rest_model::Subscription> = res
        .items
        .into_iter()
        .map(subscriptions::domain_to_rest)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(PaginatedResponse {
        data: subscriptions,
        total: res.total_results,
        offset: res.total_pages,
    })
}

pub mod rest_model {
    use uuid::Uuid;

    #[derive(serde::Serialize, serde::Deserialize)]
    pub struct Subscription {
        pub id: Uuid,
    }
}
pub mod subscriptions {
    use meteroid_store::domain;

    use crate::api::axum_routers::subscription_router::rest_model;
    use crate::errors::RestApiError;

    pub fn domain_to_rest(
        s: domain::Subscription,
    ) -> Result<rest_model::Subscription, RestApiError> {
        Ok(rest_model::Subscription { id: s.id })
    }
}
