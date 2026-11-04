use std::str::FromStr;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{WebhookDeliveryId, WebhookEndpointId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use egress_http::Url;
use error_stack::Report;
use http::StatusCode;
use meteroid_store::domain::webhooks_out::{WebhookEndpointNew, WebhookEndpointPatch};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::WebhookOutInterface;
use secrecy::ExposeSecret;

use crate::AppState;
use crate::webhooks::endpoints::mapping;
use crate::webhooks::endpoints::model::*;
use crate::webhooks::out_model::WebhookOutEventTypeEnum;
use meteroid_app::errors::RestApiError;
use meteroid_app::services::webhooks_out::{
    check_endpoint_input, check_headers, check_rate_limit, tenant_max_in_flight,
};
use meteroid_rest_common::QueryParams;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_rest_common::model::PaginationExt;

/// Same SSRF guards as the delivery worker, so a destination the worker would refuse
/// is rejected at create/update time instead.
fn validate_destination(app_state: &AppState, url: &str) -> Result<(), RestApiError> {
    let parsed =
        Url::parse(url).map_err(|e| RestApiError::InvalidInput(format!("invalid url: {e}")))?;

    app_state
        .webhook_egress
        .validate_url(&parsed)
        .map_err(|e| RestApiError::InvalidInput(format!("url is not allowed: {e}")))
}

fn check_input(
    url: Option<&str>,
    description: Option<&str>,
    event_types: Option<&[String]>,
) -> Result<(), RestApiError> {
    check_endpoint_input(url, description, event_types).map_err(RestApiError::InvalidInput)
}

fn validate_event_types(event_types: &[String]) -> Result<(), RestApiError> {
    for event_type in event_types {
        if WebhookOutEventTypeEnum::from_str(event_type).is_err() {
            return Err(RestApiError::InvalidInput(format!(
                "unknown event type `{event_type}`"
            )));
        }
    }
    Ok(())
}

/// `FailedPrecondition` (disabled endpoint) is a client error here, unlike the
/// generic store mapping which treats it as a 500.
fn store_error(context: &str, err: Report<StoreError>) -> RestApiError {
    match err.current_context() {
        StoreError::FailedPrecondition(msg) | StoreError::InvalidArgument(msg) => {
            RestApiError::InvalidInput(msg.clone())
        }
        _ => {
            log::error!("Error handling {context}: {err}");
            RestApiError::from(err)
        }
    }
}

// ── Endpoints ──────────────────────────────────────────────────

/// List webhook endpoints
#[utoipa::path(
    get,
    tag = "Webhook Endpoints",
    path = "/api/v1/webhooks/endpoints",
    responses(
        (status = 200, description = "List of webhook endpoints", body = WebhookEndpointListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn list_webhook_endpoints(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let endpoints = app_state
        .store
        .list_webhook_endpoints(authorized_state.tenant_id)
        .await
        .map_err(|e| store_error("list_webhook_endpoints", e))?;

    Ok(Json(WebhookEndpointListResponse {
        data: endpoints
            .into_iter()
            .map(mapping::endpoint_to_rest)
            .collect(),
    }))
}

/// Create a webhook endpoint
///
/// The signing secret is returned once, in this response only.
#[utoipa::path(
    post,
    tag = "Webhook Endpoints",
    path = "/api/v1/webhooks/endpoints",
    request_body(content = CreateWebhookEndpointRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Webhook endpoint created", body = CreatedWebhookEndpoint),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 409, description = "The tenant already has the maximum number of endpoints", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn create_webhook_endpoint(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<CreateWebhookEndpointRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    check_input(
        Some(&payload.url),
        payload.description.as_deref(),
        Some(&payload.event_types),
    )?;
    validate_destination(&app_state, &payload.url)?;
    validate_event_types(&payload.event_types)?;

    let headers: Vec<_> = payload
        .headers
        .into_iter()
        .map(mapping::header_from_rest)
        .collect();
    check_headers(&headers, true).map_err(RestApiError::InvalidInput)?;
    check_rate_limit(payload.rate_limit_per_sec).map_err(RestApiError::InvalidInput)?;

    let max_in_flight = tenant_max_in_flight(&app_state.store, authorized_state.tenant_id)
        .await
        .map_err(|e| store_error("resolve_webhook_max_in_flight", e))?;

    let (endpoint, secret) = app_state
        .store
        .create_webhook_endpoint(
            authorized_state.tenant_id,
            WebhookEndpointNew {
                url: payload.url,
                description: payload.description.filter(|d| !d.trim().is_empty()),
                event_types: payload.event_types,
                max_in_flight,
                headers,
                rate_limit_per_sec: payload.rate_limit_per_sec,
            },
        )
        .await
        .map_err(|e| match e.current_context() {
            StoreError::FailedPrecondition(msg) => RestApiError::FailedPrecondition(msg.clone()),
            _ => store_error("create_webhook_endpoint", e),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(CreatedWebhookEndpoint {
            endpoint: mapping::endpoint_to_rest(endpoint),
            secret: secret.expose_secret().to_string(),
        }),
    ))
}

/// Get a webhook endpoint
#[utoipa::path(
    get,
    tag = "Webhook Endpoints",
    path = "/api/v1/webhooks/endpoints/{endpoint_id}",
    params(("endpoint_id" = WebhookEndpointId, Path, description = "Webhook endpoint ID")),
    responses(
        (status = 200, description = "Webhook endpoint", body = WebhookEndpoint),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Webhook endpoint not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn get_webhook_endpoint(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(endpoint_id): Path<WebhookEndpointId>,
) -> Result<impl IntoResponse, RestApiError> {
    let endpoint = app_state
        .store
        .get_webhook_endpoint(authorized_state.tenant_id, endpoint_id)
        .await
        .map_err(|e| store_error("get_webhook_endpoint", e))?;

    Ok(Json(mapping::endpoint_to_rest(endpoint)))
}

/// Update a webhook endpoint
///
/// Omitted fields are left untouched. Re-enabling a disabled endpoint resets its
/// consecutive failure count.
#[utoipa::path(
    patch,
    tag = "Webhook Endpoints",
    path = "/api/v1/webhooks/endpoints/{endpoint_id}",
    params(("endpoint_id" = WebhookEndpointId, Path, description = "Webhook endpoint ID")),
    request_body(content = UpdateWebhookEndpointRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Webhook endpoint updated", body = WebhookEndpoint),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Webhook endpoint not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn update_webhook_endpoint(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(endpoint_id): Path<WebhookEndpointId>,
    Valid(Json(payload)): Valid<Json<UpdateWebhookEndpointRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    check_input(
        payload.url.as_deref(),
        payload.description.as_ref().and_then(|d| d.as_deref()),
        payload.event_types.as_deref(),
    )?;
    if let Some(url) = &payload.url {
        validate_destination(&app_state, url)?;
    }
    if let Some(event_types) = &payload.event_types {
        validate_event_types(event_types)?;
    }

    let headers: Option<Vec<_>> = payload
        .headers
        .map(|headers| headers.into_iter().map(mapping::header_from_rest).collect());
    if let Some(headers) = &headers {
        check_headers(headers, false).map_err(RestApiError::InvalidInput)?;
    }
    if let Some(rate) = payload.rate_limit_per_sec {
        check_rate_limit(rate).map_err(RestApiError::InvalidInput)?;
    }

    let endpoint = app_state
        .store
        .update_webhook_endpoint(
            authorized_state.tenant_id,
            WebhookEndpointPatch {
                id: endpoint_id,
                url: payload.url,
                description: payload
                    .description
                    .map(|d| d.filter(|d| !d.trim().is_empty())),
                event_types: payload.event_types,
                disabled: payload.disabled,
                headers,
                rate_limit_per_sec: payload.rate_limit_per_sec,
            },
        )
        .await
        .map_err(|e| store_error("update_webhook_endpoint", e))?;

    Ok(Json(mapping::endpoint_to_rest(endpoint)))
}

/// Delete a webhook endpoint
///
/// The endpoint is archived and its pending deliveries are cancelled.
#[utoipa::path(
    delete,
    tag = "Webhook Endpoints",
    path = "/api/v1/webhooks/endpoints/{endpoint_id}",
    params(("endpoint_id" = WebhookEndpointId, Path, description = "Webhook endpoint ID")),
    responses(
        (status = 204, description = "Webhook endpoint deleted"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Webhook endpoint not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn delete_webhook_endpoint(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(endpoint_id): Path<WebhookEndpointId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .archive_webhook_endpoint(authorized_state.tenant_id, endpoint_id)
        .await
        .map_err(|e| store_error("archive_webhook_endpoint", e))?;

    Ok(StatusCode::NO_CONTENT)
}

/// Reveal a webhook endpoint secret
#[utoipa::path(
    get,
    tag = "Webhook Endpoints",
    path = "/api/v1/webhooks/endpoints/{endpoint_id}/secret",
    params(("endpoint_id" = WebhookEndpointId, Path, description = "Webhook endpoint ID")),
    responses(
        (status = 200, description = "Signing secret", body = WebhookEndpointSecret),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Webhook endpoint not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn get_webhook_endpoint_secret(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(endpoint_id): Path<WebhookEndpointId>,
) -> Result<impl IntoResponse, RestApiError> {
    let secret = app_state
        .store
        .get_webhook_endpoint_secret(authorized_state.tenant_id, endpoint_id)
        .await
        .map_err(|e| store_error("get_webhook_endpoint_secret", e))?;

    Ok(Json(WebhookEndpointSecret {
        secret: secret.expose_secret().to_string(),
    }))
}

/// Rotate a webhook endpoint secret
///
/// The previous secret keeps signing alongside the new one for 24 hours, so consumers
/// can roll over without dropping events.
#[utoipa::path(
    post,
    tag = "Webhook Endpoints",
    path = "/api/v1/webhooks/endpoints/{endpoint_id}/rotate-secret",
    params(("endpoint_id" = WebhookEndpointId, Path, description = "Webhook endpoint ID")),
    responses(
        (status = 200, description = "New signing secret", body = WebhookEndpointSecret),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Webhook endpoint not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn rotate_webhook_endpoint_secret(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(endpoint_id): Path<WebhookEndpointId>,
) -> Result<impl IntoResponse, RestApiError> {
    let secret = app_state
        .store
        .rotate_webhook_endpoint_secret(authorized_state.tenant_id, endpoint_id)
        .await
        .map_err(|e| store_error("rotate_webhook_endpoint_secret", e))?;

    Ok(Json(WebhookEndpointSecret {
        secret: secret.expose_secret().to_string(),
    }))
}

// ── Deliveries ─────────────────────────────────────────────────

/// List deliveries for a webhook endpoint
#[utoipa::path(
    get,
    tag = "Webhook Endpoints",
    path = "/api/v1/webhooks/endpoints/{endpoint_id}/deliveries",
    params(
        ("endpoint_id" = WebhookEndpointId, Path, description = "Webhook endpoint ID"),
        DeliveryListRequest,
    ),
    responses(
        (status = 200, description = "List of deliveries", body = WebhookDeliveryListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Webhook endpoint not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn list_webhook_deliveries(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(endpoint_id): Path<WebhookEndpointId>,
    Valid(QueryParams(request)): Valid<QueryParams<DeliveryListRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .list_webhook_deliveries(
            authorized_state.tenant_id,
            endpoint_id,
            request.status.map(mapping::delivery_status_to_domain),
            request.pagination.into(),
        )
        .await
        .map_err(|e| store_error("list_webhook_deliveries", e))?;

    Ok(Json(WebhookDeliveryListResponse {
        data: res
            .items
            .into_iter()
            .map(|item| mapping::delivery_to_rest(item.delivery, item.event_type))
            .collect(),
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

/// Resend a webhook delivery
///
/// Re-queues the same event for the same endpoint. Fails if the endpoint is disabled.
#[utoipa::path(
    post,
    tag = "Webhook Endpoints",
    path = "/api/v1/webhooks/deliveries/{delivery_id}/resend",
    params(("delivery_id" = WebhookDeliveryId, Path, description = "Webhook delivery ID")),
    responses(
        (status = 200, description = "Delivery re-queued", body = WebhookDelivery),
        (status = 400, description = "Endpoint is disabled", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Delivery not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn resend_webhook_delivery(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(delivery_id): Path<WebhookDeliveryId>,
) -> Result<impl IntoResponse, RestApiError> {
    let details = app_state
        .store
        .get_webhook_delivery(authorized_state.tenant_id, delivery_id)
        .await
        .map_err(|e| store_error("get_webhook_delivery", e))?;

    let delivery = app_state
        .store
        .resend_webhook_delivery(authorized_state.tenant_id, delivery_id)
        .await
        .map_err(|e| store_error("resend_webhook_delivery", e))?;

    Ok(Json(mapping::delivery_to_rest(
        delivery,
        details.message.event_type,
    )))
}
