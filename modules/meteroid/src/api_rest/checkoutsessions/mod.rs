use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod mapping;
pub mod model;
pub mod router;

pub fn checkout_session_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::create_checkout_session))
        .routes(routes!(router::get_checkout_session))
        .routes(routes!(router::list_checkout_sessions))
        .routes(routes!(router::cancel_checkout_session))
}
