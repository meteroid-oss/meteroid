use crate::api_rest::AppState;
use axum::Router;
use axum::routing::get;

mod router;

pub use router::FileApi;

pub fn file_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/logo/{uuid}", get(router::get_logo))
        .route("/v1/invoice/pdf/{uuid}", get(router::get_invoice_pdf))
}
