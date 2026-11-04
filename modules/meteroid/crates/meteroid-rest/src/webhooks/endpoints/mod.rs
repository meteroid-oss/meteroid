use crate::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod mapping;
pub mod model;
pub mod router;

pub fn webhook_endpoint_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_webhook_endpoints))
        .routes(routes!(router::create_webhook_endpoint))
        .routes(routes!(router::get_webhook_endpoint))
        .routes(routes!(router::update_webhook_endpoint))
        .routes(routes!(router::delete_webhook_endpoint))
        .routes(routes!(router::get_webhook_endpoint_secret))
        .routes(routes!(router::rotate_webhook_endpoint_secret))
        .routes(routes!(router::list_webhook_deliveries))
        .routes(routes!(router::resend_webhook_delivery))
}
