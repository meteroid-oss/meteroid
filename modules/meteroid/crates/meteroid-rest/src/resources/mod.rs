use crate::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub(crate) mod error;
pub mod model;
mod router;

pub fn resource_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            router::list_resource_types,
            router::create_resource_type
        ))
        .routes(routes!(
            router::get_resource_type,
            router::update_resource_type
        ))
        .routes(routes!(router::list_resources))
        .routes(routes!(
            router::get_resource,
            router::upsert_resource,
            router::delete_resource
        ))
        .routes(routes!(router::archive_resource))
}
