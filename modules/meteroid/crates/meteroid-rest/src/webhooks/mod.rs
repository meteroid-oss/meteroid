use crate::AppState;
use axum::Router;
use axum::routing::post;

// `pub` so integration tests can drive the dispatcher directly (the router
// adds only signature verification + provider parsing on top of it).
pub mod endpoints;
pub use meteroid_app::payment_webhooks::event_handler;
pub mod out_model;
mod router;

pub fn webhook_in_routes() -> Router<AppState> {
    // The body cap is enforced in the handler via `to_bytes(.., MAX_WEBHOOK_BODY_BYTES)`
    // (plus the global request-body-limit layer), so no per-route limit is needed.
    Router::new()
        .route(
            "/v1/payments/{connector_id}",
            post(router::axum_payments_handler),
        )
        // Connectors created before connector-id URLs; the static `payments` segment wins over
        // this one, and a tenant id never reads `payments`.
        .route(
            "/v1/{tenant_id}/{connection_alias}",
            post(router::axum_handler),
        )
}
