use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod mapping;
mod model;
pub mod router;

pub fn product_family_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_product_families))
        .routes(routes!(router::create_product_family))
        .routes(routes!(router::get_product_family_by_id_or_alias))
}
