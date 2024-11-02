use crate::api_rest::AppState;
use axum::extract::DefaultBodyLimit;
use axum::routing::post;
use axum::Router;

mod router;

pub fn webhook_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/:provider/:endpoint_uid",
            post(crate::api_rest::webhooks::router::axum_handler),
        )
        .layer(DefaultBodyLimit::max(4096))
}
