use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod mapping;
mod model;
pub mod router;

pub fn plan_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_plans))
        .routes(routes!(router::create_plan))
        .routes(routes!(router::get_plan_details))
        .routes(routes!(router::replace_plan))
        .routes(routes!(router::patch_plan))
        .routes(routes!(router::publish_plan))
        .routes(routes!(router::archive_plan))
        .routes(routes!(router::unarchive_plan))
        .routes(routes!(router::list_plan_versions))
}
