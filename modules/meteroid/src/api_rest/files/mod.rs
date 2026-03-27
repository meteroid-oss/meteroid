use crate::api_rest::AppState;
use axum::Router;
use axum::routing::get;

mod router;

pub fn file_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/logo/{uuid}", get(router::get_logo))
        .route("/v1/invoice/pdf/{uuid}", get(router::get_invoice_pdf))
        .route(
            "/v1/batch-job/errors/{batch_job_id}",
            get(router::get_batch_job_error_csv),
        )
        .route(
            "/v1/batch-job/input/{batch_job_id}",
            get(router::get_batch_job_input_file),
        )
}
