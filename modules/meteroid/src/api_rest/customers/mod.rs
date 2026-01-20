use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod mapping;
pub mod model;
mod router;

pub fn customer_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_customers))
        .routes(routes!(router::get_customer))
        .routes(routes!(router::create_customer))
        .routes(routes!(router::update_customer))
        .routes(routes!(router::patch_customer))
        .routes(routes!(router::archive_customer))
        .routes(routes!(router::unarchive_customer))
        .routes(routes!(router::create_portal_token))
}
