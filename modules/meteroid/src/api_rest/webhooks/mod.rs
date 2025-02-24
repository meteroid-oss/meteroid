use crate::api_rest::AppState;
use axum::extract::DefaultBodyLimit;
use axum::routing::post;
use axum::Router;

mod router;

pub fn webhook_in_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/{provider}/{tenant_id}", post(router::axum_handler))
        .layer(DefaultBodyLimit::max(4096))
}
