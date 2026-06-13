use crate::api_rest::AppState;
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::post;

// `pub` so integration tests can drive the dispatcher directly (the router
// adds only signature verification + provider parsing on top of it).
pub mod event_handler;
pub mod out_model;
mod router;

pub fn webhook_in_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/{tenant_id}/{connection_alias}",
            post(router::axum_handler),
        )
        .layer(DefaultBodyLimit::max(4096))
}
