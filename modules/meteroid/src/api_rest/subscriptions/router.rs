use super::AppState;
use std::str::FromStr;

use axum::extract::Path;
use axum::{Json, extract::State, response::IntoResponse};

use crate::api_rest::QueryParams;
use crate::api_rest::error::RestErrorResponse;
use crate::api_rest::model::PaginationExt;
use crate::api_rest::subscriptions::mapping::{
    domain_to_rest, domain_to_rest_details, rest_to_domain_create_request,
    rest_to_domain_update_request,
};
use crate::api_rest::subscriptions::model::{
    MetricUsageSummary, Subscription, SubscriptionCreateRequest, SubscriptionDetails,
    SubscriptionListResponse, SubscriptionRequest, SubscriptionUpdateRequest,
    SubscriptionUpdateResponse, SubscriptionUsageResponse, UsageDataPointRest,
};
use crate::errors::RestApiError;
use axum::Extension;
use axum_valid::Valid;
use common_domain::ids::{AliasOr, BaseId, CustomerId, SubscriptionId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use itertools::Itertools;
use meteroid_store::repositories::coupons::CouponInterface;
use meteroid_store::repositories::subscriptions::{
    CancellationEffectiveAt, SubscriptionInterfaceAuto,
};
use meteroid_store::repositories::{CustomersInterface, PlansInterface, SubscriptionInterface};
use rust_decimal::Decimal;

/// List subscriptions
///
/// List subscriptions with optional filtering by customer or plan.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v1/subscriptions",
    params(
        SubscriptionRequest
    ),
    responses(
        (status = 200, description = "List of subscriptions", body = SubscriptionListResponse),
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
    Valid(QueryParams(request)): Valid<QueryParams<SubscriptionRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let status_filter = request
        .statuses
        .map(|statuses| statuses.into_iter().map(|s| s.into()).collect());

    let customer_id = match request.customer_id {
        None => None,
        Some(c) => match c {
            AliasOr::Id(id) => Some(id),
            AliasOr::Alias(alias) => app_state
                .store
                .find_customer_by_alias(alias, authorized_state.tenant_id)
                .await
                .map_err(|e| {
                    log::error!(
                        "Error handling get_customer for tenant {}: {} ",
                        authorized_state.tenant_id.as_uuid(),
                        e
                    );
                    RestApiError::from(e)
                })
                .map(|c| Some(c.id))?,
        },
    };

    let res = app_state
        .store
        .list_subscriptions(
            authorized_state.tenant_id,
            customer_id,
            request.plan_id,
            status_filter,
            request.pagination.into(),
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_subscriptions: {e}");
            RestApiError::from(e)
        })?;

    let subscriptions: Vec<Subscription> = res
        .items
        .into_iter()
        .map(domain_to_rest)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(SubscriptionListResponse {
        data: subscriptions,
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

/// Get subscription details
///
/// Retrieve detailed information about a subscription including price components and schedules.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v1/subscriptions/{subscription_id}",
    params(
        ("subscription_id" = SubscriptionId, Path, description = "Subscription ID")
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
    Path(subscription_id): Path<SubscriptionId>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .get_subscription_details(authorized_state.tenant_id, subscription_id)
        .await
        .map_err(|e| {
            log::error!("Error handling subscription_details: {e}");
            RestApiError::from(e)
        })
        .and_then(domain_to_rest_details)?;

    Ok(Json(res))
}

/// Create subscription
///
/// Create a new subscription for a customer with a specific plan.
#[utoipa::path(
    post,
    tag = "Subscriptions",
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
                .find_customer_id_by_alias(alias, authorized_state.tenant_id)
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
                    log::error!("Error resolving coupon codes: {e}");
                    RestApiError::from(e)
                })?
                .into_iter()
                .map(|c| c.id)
                .collect_vec(),
        ),
        _ => None,
    };

    let resolved_plan_version = app_state
        .store
        .resolve_published_version_id(payload.plan_id, payload.version, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error resolving plan version: {}", e);
            RestApiError::from(e)
        })?;

    let created = app_state
        .services
        .insert_subscription(
            rest_to_domain_create_request(
                resolved_plan_version,
                resolved_customer_id,
                resolved_coupon_ids,
                authorized_state.actor_id,
                payload,
            )?,
            authorized_state.tenant_id,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling create subscription request: {e}");
            RestApiError::from(e)
        })?;

    let res = app_state
        .store
        .get_subscription_details(authorized_state.tenant_id, created.id)
        .await
        .map_err(|e| {
            log::error!("Error handling subscription_details: {e}");
            RestApiError::from(e)
        })
        .and_then(domain_to_rest_details)?;

    Ok((StatusCode::CREATED, Json(res)))
}

/// Cancel subscription
///
/// Cancel a subscription either immediately or at the end of the billing period.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v1/subscriptions/{subscription_id}/cancel",
    params(
        ("subscription_id" = SubscriptionId, Path, description = "Subscription ID", example = "sub_123"),
    ),
    request_body = super::model::CancelSubscriptionRequest,
    responses(
        (status = 200, description = "Subscription canceled", body = super::model::CancelSubscriptionResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Subscription not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn cancel_subscription(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(subscription_id): Path<SubscriptionId>,
    Valid(Json(request)): Valid<Json<super::model::CancelSubscriptionRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let subscription = app_state
        .services
        .cancel_subscription(
            subscription_id,
            authorized_state.tenant_id,
            request.reason.clone(),
            request
                .effective_date
                .map(CancellationEffectiveAt::Date)
                .unwrap_or(CancellationEffectiveAt::EndOfBillingPeriod),
            authorized_state.actor_id,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling cancel_subscription: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(super::model::CancelSubscriptionResponse {
        subscription: domain_to_rest(subscription)?,
    }))
}

/// Update subscription
///
/// Update subscription settings like payment configuration, billing options, etc.
#[utoipa::path(
    patch,
    tag = "Subscriptions",
    path = "/api/v1/subscriptions/{subscription_id}",
    params(
        ("subscription_id" = SubscriptionId, Path, description = "Subscription ID"),
    ),
    request_body = SubscriptionUpdateRequest,
    responses(
        (status = 200, description = "Subscription updated", body = SubscriptionUpdateResponse),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Subscription not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn update_subscription(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(subscription_id): Path<SubscriptionId>,
    Valid(Json(request)): Valid<Json<SubscriptionUpdateRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let patch = rest_to_domain_update_request(subscription_id, request);

    app_state
        .store
        .patch_subscription(authorized_state.tenant_id, patch)
        .await
        .map_err(|e| {
            log::error!("Error handling update_subscription: {e}");
            RestApiError::from(e)
        })?;

    let details = app_state
        .store
        .get_subscription_details(authorized_state.tenant_id, subscription_id)
        .await
        .map_err(|e| {
            log::error!("Error fetching updated subscription: {e}");
            RestApiError::from(e)
        })
        .and_then(domain_to_rest_details)?;

    Ok(Json(SubscriptionUpdateResponse {
        subscription: details,
    }))
}

/// Get subscription usage
///
/// Retrieve usage data for all usage-based components of a subscription in the current billing period.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v1/subscriptions/{subscription_id}/usage",
    params(
        ("subscription_id" = SubscriptionId, Path, description = "Subscription ID"),
    ),
    responses(
        (status = 200, description = "Subscription usage data", body = SubscriptionUsageResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Subscription not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn get_subscription_usage(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(subscription_id): Path<SubscriptionId>,
) -> Result<impl IntoResponse, RestApiError> {
    let details = app_state
        .store
        .get_subscription_details(authorized_state.tenant_id, subscription_id)
        .await
        .map_err(|e| {
            log::error!("Error fetching subscription details for usage: {e}");
            RestApiError::from(e)
        })?;

    let period_start = details.subscription.current_period_start;
    let period_end = details
        .subscription
        .current_period_end
        .unwrap_or_else(|| chrono::Utc::now().date_naive() + chrono::Duration::days(1));

    // Collect usage-based metrics from components and add-ons
    let usage_metric_ids: Vec<_> = details.metrics.iter().map(|m| m.id).collect();

    let mut metrics = Vec::new();
    for metric_id in usage_metric_ids {
        let usage = app_state
            .services
            .get_subscription_component_usage(&details, metric_id)
            .await
            .map_err(|e| {
                log::error!("Error fetching usage for metric {metric_id}: {e}");
                RestApiError::from(e)
            })?;

        let metric = details
            .metrics
            .iter()
            .find(|m| m.id == metric_id)
            .ok_or(RestApiError::NotFound)?;

        let total_value = usage
            .data
            .iter()
            .fold(Decimal::ZERO, |acc, p| acc + p.value);

        metrics.push(MetricUsageSummary {
            metric_id,
            metric_name: metric.name.clone(),
            metric_code: metric.code.clone(),
            total_value,
            data_points: usage
                .data
                .into_iter()
                .map(|p| UsageDataPointRest {
                    window_start: p.window_start,
                    window_end: p.window_end,
                    value: p.value,
                    dimensions: p.dimensions,
                })
                .collect(),
        });
    }

    Ok(Json(SubscriptionUsageResponse {
        subscription_id,
        period_start,
        period_end,
        metrics,
    }))
}
