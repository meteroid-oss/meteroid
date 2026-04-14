use crate::services::svix_cache::SvixEndpointCache;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use common_domain::ids::TenantId;
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Path segment where our Axum router mounts the handler. Shared with the
/// boot-time registration check so they can't drift.
pub const OP_WEBHOOK_PATH: &str = "/webhooks/svix-operational";

// Endpoint is public (signature-protected), so unbounded logs on bad sigs = DoS vector.
const SIG_FAIL_LOG_INTERVAL_SECS: u64 = 30;

#[derive(Clone)]
pub struct SvixOperationalState {
    pub webhook_verifier: Arc<svix::webhooks::Webhook>,
    pub endpoint_cache: Arc<dyn SvixEndpointCache>,
    pub sig_fail_last_log: Arc<AtomicU64>,
    pub sig_fail_suppressed: Arc<AtomicU64>,
}

impl SvixOperationalState {
    pub fn new(
        webhook_verifier: Arc<svix::webhooks::Webhook>,
        endpoint_cache: Arc<dyn SvixEndpointCache>,
    ) -> Self {
        Self {
            webhook_verifier,
            endpoint_cache,
            sig_fail_last_log: Arc::new(AtomicU64::new(0)),
            sig_fail_suppressed: Arc::new(AtomicU64::new(0)),
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
    app_uid: Option<String>,
}

const ENDPOINT_EVENTS: &[&str] = &[
    "endpoint.created",
    "endpoint.deleted",
    "endpoint.disabled",
    "endpoint.enabled",
    "endpoint.updated",
];

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn log_signature_failure(state: &SvixOperationalState, err: impl std::fmt::Display) {
    let now = now_secs();
    let last = state.sig_fail_last_log.load(Ordering::Relaxed);

    if now.saturating_sub(last) >= SIG_FAIL_LOG_INTERVAL_SECS
        && state
            .sig_fail_last_log
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        let suppressed = state.sig_fail_suppressed.swap(0, Ordering::Relaxed);
        if suppressed > 0 {
            tracing::warn!(
                "Svix operational webhook signature verification failed: {err} \
                 ({suppressed} similar failures suppressed in the last {SIG_FAIL_LOG_INTERVAL_SECS}s)"
            );
        } else {
            tracing::warn!("Svix operational webhook signature verification failed: {err}");
        }
    } else {
        state.sig_fail_suppressed.fetch_add(1, Ordering::Relaxed);
    }
}

async fn handle_svix_operational(
    State(state): State<SvixOperationalState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    if let Err(e) = state.webhook_verifier.verify(&body, &headers) {
        log_signature_failure(&state, e);
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
