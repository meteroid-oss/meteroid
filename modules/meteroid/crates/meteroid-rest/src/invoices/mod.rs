use crate::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub mod mapping;
pub mod model;
pub mod router;

pub fn invoice_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_invoices))
        .routes(routes!(router::get_invoice_by_id))
        .routes(routes!(router::patch_invoice_custom_properties))
        .routes(routes!(router::refresh_invoice))
        .routes(routes!(router::download_invoice_pdf))
        .routes(routes!(router::download_invoice_xml))
}
