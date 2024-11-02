use crate::api_rest::AppState;
use axum::routing::get;
use axum::Router;

mod file_router;

pub fn file_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/logo/:uuid", get(file_router::get_logo))
        .route("/v1/invoice/pdf/:uuid", get(file_router::get_invoice_pdf))
}

pub use file_router::FileApi;
