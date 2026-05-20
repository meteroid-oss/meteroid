use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod mapping;
pub mod model;
pub mod router;

pub fn product_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_products))
        .routes(routes!(router::create_product))
        .routes(routes!(router::get_product))
        .routes(routes!(router::update_product))
        .routes(routes!(router::archive_product))
        .routes(routes!(router::unarchive_product))
}
