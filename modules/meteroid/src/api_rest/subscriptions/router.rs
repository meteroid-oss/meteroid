use super::AppState;
use std::str::FromStr;

use axum::extract::{Path, Query};
use axum::{Json, extract::State, response::IntoResponse};

use crate::api_rest::error::RestErrorResponse;
use crate::api_rest::model::PaginatedResponse;
use crate::api_rest::subscriptions::mapping::{
    domain_to_rest, domain_to_rest_details, rest_to_domain_create_request,
};
use crate::api_rest::subscriptions::model::{
    Subscription, SubscriptionCreateRequest, SubscriptionDetails, SubscriptionRequest,
};
use crate::errors::RestApiError;
use axum::Extension;
use axum_valid::Valid;
use common_domain::ids::{AliasOr, CustomerId, PlanId, SubscriptionId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use itertools::Itertools;
use meteroid_store::repositories::coupons::CouponInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use meteroid_store::repositories::{CustomersInterface, SubscriptionInterface};

#[utoipa::path(
    get,
    tag = "subscription",
    path = "/api/v1/subscriptions",
    params(
        ("page" = usize, Query, description = "Specifies the starting position of the results", example = 0, minimum = 0),
        ("per_page" = usize, Query, description = "The maximum number of objects to return", example = 10, minimum = 1, maximum = 100),
        ("customer_id" = CustomerId, Query, description = "Filter by customer ID"),
        ("plan_id" = PlanId, Query, description = "Filter by plan ID")
    ),
    responses(
        (status = 200, description = "List of subscriptions", body = PaginatedResponse<Subscription>),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn list_subscriptions(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(Query(request)): Valid<Query<SubscriptionRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .list_subscriptions(
            authorized_state.tenant_id,
            request.customer_id,
            request.plan_id,
            request.pagination.into(),
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_subscriptions: {}", e);
            RestApiError::from(e)
        })?;

    let subscriptions: Vec<Subscription> = res
        .items
        .into_iter()
        .map(domain_to_rest)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(PaginatedResponse {
        data: subscriptions,
        total: res.total_results,
    }))
}

#[utoipa::path(
    get,
    tag = "subscription",
    path = "/api/v1/subscriptions/{id}",
    params(
        ("id" = SubscriptionId, Path, description = "subscription ID")
    ),
    responses(
        (status = 200, description = "Details of subscription", body = SubscriptionDetails),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Subscription not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn subscription_details(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(id): Path<SubscriptionId>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .get_subscription_details(authorized_state.tenant_id, id)
        .await
        .map_err(|e| {
            log::error!("Error handling subscription_details: {}", e);
            RestApiError::from(e)
        })
        .and_then(domain_to_rest_details)?;

    Ok(Json(res))
}

#[utoipa::path(
    post,
    tag = "subscription",
    path = "/api/v1/subscriptions",
    request_body(content = SubscriptionCreateRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Subscription successfully created", body = SubscriptionDetails),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn create_subscription(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<SubscriptionCreateRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let id_or_alias: AliasOr<CustomerId> = AliasOr::from_str(payload.customer_id_or_alias.as_str())
        .map_err(|_| RestApiError::InvalidInput)?;

    let resolved_customer_id = match id_or_alias {
        AliasOr::Id(id) => id,
        AliasOr::Alias(alias) => {
            app_state
                .store
                .find_customer_by_alias(alias)
                .await
                .map_err(RestApiError::from)?
                .id
        }
    };

    let resolved_coupon_ids = match payload.coupon_codes.as_ref() {
        Some(codes) if !codes.is_empty() => Some(
            app_state
                .store
                .list_coupons_by_codes(authorized_state.tenant_id, codes)
                .await
                .map_err(|e| {
                    log::error!("Error resolving coupon codes: {}", e);
                    RestApiError::from(e)
                })?
                .into_iter()
                .map(|c| c.id)
                .collect_vec(),
        ),
        _ => None,
    };

    let created = app_state
        .services
        .insert_subscription(
            rest_to_domain_create_request(
                resolved_customer_id,
                resolved_coupon_ids,
                authorized_state.actor_id,
                payload,
            )?,
            authorized_state.tenant_id,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling create subscription request: {}", e);
            RestApiError::from(e)
        })?;

    let res = app_state
        .store
        .get_subscription_details(authorized_state.tenant_id, created.id)
        .await
        .map_err(|e| {
            log::error!("Error handling subscription_details: {}", e);
            RestApiError::from(e)
        })
        .and_then(domain_to_rest_details)?;

    Ok((StatusCode::CREATED, Json(res)))
}
