use crate::api_rest::AppState;
use axum::Router;
use axum::routing::get;

mod return_handler;

pub fn stancer_routes() -> Router<AppState> {
    Router::new().route("/v1/portal/stancer/return", get(return_handler::handle))
}
