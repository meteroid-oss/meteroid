use crate::api_rest::AppState;
use axum::Router;
use axum::routing::post;

pub mod out_model;
mod router;

pub fn webhook_in_routes() -> Router<AppState> {
    // The body cap is enforced in the handler via `to_bytes(.., MAX_WEBHOOK_BODY_BYTES)`
    // (plus the global request-body-limit layer), so no per-route limit is needed.
    Router::new().route(
        "/v1/{tenant_id}/{connection_alias}",
        post(router::axum_handler),
    )
}
