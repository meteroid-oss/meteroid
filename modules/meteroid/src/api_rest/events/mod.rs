pub mod mapping;
pub mod model;
pub mod router;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::api_rest::AppState;

pub fn event_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(router::ingest_events))
}
