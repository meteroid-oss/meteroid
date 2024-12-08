use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod mapping;
mod model;
pub mod router;

pub fn plan_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(router::list_plans))
}
