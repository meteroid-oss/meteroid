use crate::services::svix_cache::SvixEndpointCache;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use common_domain::ids::TenantId;
use serde::Deserialize;
use std::sync::Arc;

pub const OP_WEBHOOK_PATH: &str = "/webhooks/svix-operational";

#[derive(Clone)]
pub struct SvixOperationalState {
    pub webhook_verifier: Arc<svix::webhooks::Webhook>,
    pub endpoint_cache: Arc<dyn SvixEndpointCache>,
}

impl SvixOperationalState {
    pub fn new(
        webhook_verifier: Arc<svix::webhooks::Webhook>,
        endpoint_cache: Arc<dyn SvixEndpointCache>,
    ) -> Self {
        Self {
            webhook_verifier,
            endpoint_cache,
        }
    }
}

pub fn svix_operational_routes(state: SvixOperationalState) -> Router {
    Router::new()
        .route("/", post(handle_svix_operational))
        .with_state(state)
}

#[derive(Deserialize)]
struct OperationalWebhookPayload {
    #[serde(rename = "type")]
    event_type: String,
    data: OperationalWebhookData,
}

#[derive(Deserialize)]
struct OperationalWebhookData {
    #[serde(rename = "appUid")]
    app_uid: Option<String>,
}

const ENDPOINT_EVENTS: &[&str] = &[
    "endpoint.created",
    "endpoint.deleted",
    "endpoint.disabled",
    "endpoint.enabled",
    "endpoint.updated",
];

async fn handle_svix_operational(
    State(state): State<SvixOperationalState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    if let Err(e) = state.webhook_verifier.verify(&body, &headers) {
        tracing::warn!("Svix operational webhook signature verification failed: {e}");
        return StatusCode::BAD_REQUEST;
    }

    let payload: OperationalWebhookPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("Failed to parse Svix operational webhook payload: {e}");
            return StatusCode::BAD_REQUEST;
        }
    };

    if !ENDPOINT_EVENTS.contains(&payload.event_type.as_str()) {
        return StatusCode::NO_CONTENT;
    }

    let Some(app_uid) = payload.data.app_uid else {
        tracing::warn!(
            "Svix operational webhook {} missing app_uid",
            payload.event_type
        );
        return StatusCode::NO_CONTENT;
    };

    let tenant_id: TenantId = match app_uid.parse() {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!("Failed to parse app_uid as TenantId: {e}");
            return StatusCode::NO_CONTENT;
        }
    };

    tracing::info!(
        "Svix operational webhook {}: invalidating endpoint cache for tenant {tenant_id}",
        payload.event_type
    );
    state.endpoint_cache.invalidate(&tenant_id).await;

    StatusCode::OK
}
