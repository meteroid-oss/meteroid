use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub mod mapping;
pub mod model;
pub mod router;

pub fn subscription_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::create_subscription))
        .routes(routes!(router::list_subscriptions))
        .routes(routes!(router::subscription_details))
        .routes(routes!(router::cancel_subscription))
        .routes(routes!(router::update_subscription))
}
