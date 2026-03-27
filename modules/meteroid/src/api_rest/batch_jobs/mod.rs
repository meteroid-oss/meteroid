pub mod model;
pub mod router;

use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub fn batch_job_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_batch_jobs))
        .routes(routes!(router::get_batch_job))
        .routes(routes!(router::list_batch_job_failures))
}
