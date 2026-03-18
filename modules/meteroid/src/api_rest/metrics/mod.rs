use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod mapping;
pub mod model;
pub mod router;

pub fn metric_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_metrics))
        .routes(routes!(router::create_metric))
        .routes(routes!(router::get_metric))
        .routes(routes!(router::update_metric))
        .routes(routes!(router::archive_metric))
        .routes(routes!(router::unarchive_metric))
}
