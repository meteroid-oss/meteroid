use crate::api_rest::AppState;
use axum::routing::get;
use axum::Router;

mod mapping;
mod model;
pub mod subscription_router;

pub fn subscription_routes() -> Router<AppState> {
    Router::new().route(
        "/v1/subscriptions",
        get(subscription_router::list_subscriptions),
    )
}
