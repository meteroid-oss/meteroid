use crate::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod mapping;
pub mod model;
pub mod router;

pub fn credit_note_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_credit_notes))
        .routes(routes!(router::get_credit_note_by_id))
        .routes(routes!(router::patch_credit_note_custom_properties))
        .routes(routes!(router::download_credit_note_pdf))
        .routes(routes!(router::download_credit_note_xml))
}
