use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub mod router;

pub fn credit_note_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(router::download_credit_note_pdf))
}
