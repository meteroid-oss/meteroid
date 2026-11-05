use super::AppState;
use std::str::FromStr;

use axum::extract::Path;
use axum::{Json, extract::State, response::IntoResponse};

use crate::subscriptions::mapping::{
    domain_to_rest, domain_to_rest_details, rest_to_domain_create_request,
    rest_to_domain_update_request,
};
use crate::subscriptions::model::{
    Subscription, SubscriptionCreateRequest, SubscriptionDetails, SubscriptionListResponse,
    SubscriptionRequest, SubscriptionUpdateRequest, SubscriptionUpdateResponse,
};
use axum::Extension;
use axum_valid::Valid;
use common_domain::ids::{AliasOr, BaseId, CustomerId, SubscriptionId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_app::errors::RestApiError;
use meteroid_app::features::ensure_credit_pricing_allowed;
use meteroid_rest_common::QueryParams;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_rest_common::model::{PaginationExt, validate_order_by};
use meteroid_store::repositories::coupons::CouponInterface;
use meteroid_store::repositories::subscriptions::{
    CancellationEffectiveAt, SubscriptionInterfaceAuto,
};
use meteroid_store::repositories::{CustomersInterface, PlansInterface, SubscriptionInterface};

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
pub async fn list_subscriptions(
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
                .find_customer_by_alias(alias.clone(), authorized_state.tenant_id)
                .await
                .map_err(|e| {
                    log::warn!(
                        "list_subscriptions: failed to resolve customer alias '{alias}' for tenant {}: {e}",
                        authorized_state.tenant_id.as_uuid()
                    );
                    RestApiError::from(e)
                })
                .map(|c| Some(c.id))?,
        },
    };

    let order_by = validate_order_by(
        &request.order_by,
        &[
            "customer_name",
            "plan_name",
            "mrr_cents",
            "billing_start_date",
            "end_date",
            "status",
            "created_at",
        ],
        "created_at.desc",
    )
    .map_err(RestApiError::InvalidInput)?;

    let res = app_state
        .store
        .list_subscriptions(
            authorized_state.tenant_id,
            customer_id,
            request.plan_id,
            status_filter,
            None,
            vec![],
            Some(order_by),
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
pub async fn subscription_details(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(subscription_id): Path<SubscriptionId>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .get_subscription_details(authorized_state.tenant_id, subscription_id)
        .await
        .map_err(|e| {
            log::error!(
                "subscription_details: failed to fetch subscription {subscription_id}: {e}"
            );
            RestApiError::from(e)
        })
        .and_then(domain_to_rest_details)?;

    Ok(Json(res))
}

/// Get subscription summary
///
/// Retrieve a subscription without its components, add-ons, coupons and entitlements: the same
/// shape as list items, for callers that only need status and billing dates.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v1/subscriptions/{subscription_id}/summary",
    params(
        ("subscription_id" = SubscriptionId, Path, description = "Subscription ID")
    ),
    responses(
        (status = 200, description = "Subscription summary", body = Subscription),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Subscription not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub async fn subscription_summary(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(subscription_id): Path<SubscriptionId>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .get_subscription(authorized_state.tenant_id, subscription_id)
        .await
        .map_err(|e| {
            log::error!(
                "subscription_summary: failed to fetch subscription {subscription_id}: {e}"
            );
            RestApiError::from(e)
        })
        .and_then(domain_to_rest)?;

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
pub async fn create_subscription(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<SubscriptionCreateRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let id_or_alias: AliasOr<CustomerId> = AliasOr::from_str(payload.customer_id_or_alias.as_str())
        .map_err(|_| RestApiError::InvalidInput("invalid customer_id format".to_string()))?;

    let resolved_customer_id = match id_or_alias {
        AliasOr::Id(id) => id,
        AliasOr::Alias(alias) => {
            app_state
                .store
                .find_customer_id_by_alias(alias.clone(), authorized_state.tenant_id)
                .await
                .map_err(|e| {
                    log::warn!(
                        "create_subscription: failed to resolve customer alias '{alias}' for tenant {}: {e}",
                        authorized_state.tenant_id.as_uuid()
                    );
                    RestApiError::from(e)
                })?
                .id
        }
    };

    let resolved_coupon_ids = match payload.coupon_codes.as_ref() {
        Some(codes) if !codes.is_empty() => Some(
            app_state
                .store
                .resolve_coupon_codes(authorized_state.tenant_id, codes)
                .await
                .map_err(RestApiError::from)?,
        ),
        _ => None,
    };

    let resolved_plan_version = app_state
        .store
        .resolve_published_version_id(payload.plan_id, payload.version, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!(
                "create_subscription: failed to resolve published version for plan {} (version {:?}): {e}",
                payload.plan_id,
                payload.version
            );
            RestApiError::from(e)
        })?;

    let create = rest_to_domain_create_request(
        resolved_plan_version,
        resolved_customer_id,
        resolved_coupon_ids,
        payload,
    )?;
    ensure_credit_pricing_allowed(
        app_state.feature_resolver.as_ref(),
        authorized_state.organization_id,
        create.price_components.as_ref().is_some_and(|components| {
            components
                .extra_components
                .iter()
                .any(|e| e.product_ref.creates_credit_product())
        }),
    )
    .await?;

    let created = app_state
        .services
        .insert_subscription(authorized_state.as_actor(), create, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!(
                "create_subscription: failed to insert subscription for customer {resolved_customer_id}: {e}"
            );
            RestApiError::from(e)
        })?;

    let res = app_state
        .store
        .get_subscription_details(authorized_state.tenant_id, created.id)
        .await
        .map_err(|e| {
            log::error!(
                "create_subscription: failed to fetch created subscription {}: {e}",
                created.id
            );
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
pub async fn cancel_subscription(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(subscription_id): Path<SubscriptionId>,
    Valid(Json(request)): Valid<Json<super::model::CancelSubscriptionRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let subscription = app_state
        .services
        .cancel_subscription(
            authorized_state.as_actor(),
            subscription_id,
            authorized_state.tenant_id,
            request.reason.clone(),
            request
                .effective_date
                .map(CancellationEffectiveAt::Date)
                .unwrap_or(CancellationEffectiveAt::EndOfBillingPeriod),
        )
        .await
        .map_err(|e| {
            log::error!(
                "cancel_subscription: failed to cancel subscription {subscription_id}: {e}"
            );
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
pub async fn update_subscription(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(subscription_id): Path<SubscriptionId>,
    Valid(Json(request)): Valid<Json<SubscriptionUpdateRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let patch = rest_to_domain_update_request(subscription_id, request)?;

    app_state
        .store
        .patch_subscription(
            authorized_state.tenant_id,
            patch,
            authorized_state.as_actor(),
        )
        .await
        .map_err(|e| {
            log::error!("update_subscription: failed to patch subscription {subscription_id}: {e}");
            RestApiError::from(e)
        })?;

    let details = app_state
        .store
        .get_subscription_details(authorized_state.tenant_id, subscription_id)
        .await
        .map_err(|e| {
            log::error!(
                "update_subscription: failed to fetch updated subscription {subscription_id}: {e}"
            );
            RestApiError::from(e)
        })
        .and_then(domain_to_rest_details)?;

    Ok(Json(SubscriptionUpdateResponse {
        subscription: details,
    }))
}
