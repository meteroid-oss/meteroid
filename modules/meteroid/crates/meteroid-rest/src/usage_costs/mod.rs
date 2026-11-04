use crate::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub mod model;
pub mod router;

pub fn usage_cost_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_customer_usage_events))
        .routes(routes!(router::list_customer_usage_costs))
        .routes(routes!(router::list_subscription_usage_events))
        .routes(routes!(router::list_subscription_usage_costs))
}
