use crate::api_rest::AppState;
use axum::routing::get;
use axum::Router;

mod router;

pub use router::FileApi;

pub fn file_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/logo/:uuid", get(router::get_logo))
        .route("/v1/invoice/pdf/:uuid", get(router::get_invoice_pdf))
}
