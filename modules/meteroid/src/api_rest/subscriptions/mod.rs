use crate::api_rest::AppState;
use axum::routing::get;
use axum::Router;

mod mapping;
mod model;
pub mod router;

pub use router::SubscriptionApi;

pub fn subscription_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/subscriptions", get(router::list_subscriptions))
        .route("/v1/subscription/:uuid", get(router::subscription_details))
}
