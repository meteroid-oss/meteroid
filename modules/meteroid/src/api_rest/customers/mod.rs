use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod mapping;
mod model;
mod router;

pub fn customer_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_customers))
        .routes(routes!(router::get_customer_by_id_or_alias))
}
