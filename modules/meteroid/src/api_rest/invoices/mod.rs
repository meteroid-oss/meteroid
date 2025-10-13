use crate::api_rest::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub mod model;
pub mod router;
mod mapping;

pub fn invoice_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(router::list_invoices))
        .routes(routes!(router::get_invoice_by_id))
}
