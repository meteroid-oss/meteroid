use crate::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod router;

/// Reads only: definitions are managed by the Enterprise `custom-properties` module.
pub fn custom_property_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_definitions))
        .routes(routes!(router::get_definition))
}
