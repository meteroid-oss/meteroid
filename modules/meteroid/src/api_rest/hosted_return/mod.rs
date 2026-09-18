use crate::api_rest::AppState;
use axum::Router;
use axum::routing::get;

mod return_handler;

/// One return handler for all hosted-redirect providers. The provider-named paths are kept for
/// return URLs already embedded in in-flight intents.
pub fn hosted_return_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/portal/hosted/return", get(return_handler::handle))
        .route("/v1/portal/gocardless/return", get(return_handler::handle))
        .route("/v1/portal/stancer/return", get(return_handler::handle))
        .route("/v1/portal/mollie/return", get(return_handler::handle))
}
