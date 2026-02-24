pub mod model;
pub mod router;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::api_rest::AppState;

pub fn usage_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::get_customer_usage))
        .routes(routes!(router::get_subscription_usage))
        .routes(routes!(router::get_usage_summary))
}
