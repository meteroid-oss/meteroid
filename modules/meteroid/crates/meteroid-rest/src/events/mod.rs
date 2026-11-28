pub mod mapping;
pub mod model;
pub mod router;

pub use router::ingest_checked;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;

pub fn event_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(router::ingest_events))
}
