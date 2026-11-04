use crate::AppState;
use axum::Router;
use axum::routing::get;

pub mod router;

pub use router::{no_xml, xml_download};

pub fn file_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/logo/{uuid}", get(router::get_logo))
        .route("/v1/invoice/pdf/{uuid}", get(router::get_invoice_pdf))
        .route("/v1/invoice/xml/{uuid}", get(router::get_invoice_xml))
        .route(
            "/v1/credit-note/pdf/{uuid}",
            get(router::get_credit_note_pdf),
        )
        .route(
            "/v1/credit-note/xml/{uuid}",
            get(router::get_credit_note_xml),
        )
        .route(
            "/v1/email/attachment/{activity_id}/{doc_id}",
            get(router::get_email_attachment),
        )
        .route(
            "/v1/batch-job/errors/{batch_job_id}",
            get(router::get_batch_job_error_csv),
        )
        .route(
            "/v1/batch-job/input/{batch_job_id}",
            get(router::get_batch_job_input_file),
        )
        .route(
            "/v1/tax-report/output/{batch_job_id}",
            get(router::get_batch_job_output),
        )
        .route(
            "/v1/credit-ledger/{customer_id}",
            get(router::get_credit_ledger_csv),
        )
}
