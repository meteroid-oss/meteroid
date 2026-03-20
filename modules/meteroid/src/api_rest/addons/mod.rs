use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod mapping;
pub mod model;
pub mod router;

pub fn addon_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_addons))
        .routes(routes!(router::create_addon))
        .routes(routes!(router::get_addon))
        .routes(routes!(router::update_addon))
        .routes(routes!(router::archive_addon))
        .routes(routes!(router::unarchive_addon))
}
