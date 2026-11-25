//! `/api/v2/invoices` (ADR-0013 §12): invoices listed by keyset in SQL, read whole with their
//! lines as the engine wrote them (one rendering with the documents a subscription change
//! previews, `v2::documents`), their files, and what v1 and the dashboard do to them, each on the
//! same store service: a one-off invoice written, a draft's lines edited (`write`), a draft
//! issued, a payment recorded (`actions`).
use crate::AppState;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod actions;
pub mod model;
pub(crate) mod read;
mod write;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(read::list_invoices, write::create_invoice))
        .routes(routes!(read::get_invoice, write::update_invoice))
        .routes(routes!(read::download_invoice_pdf))
        .routes(routes!(read::download_invoice_xml))
        .routes(routes!(actions::refresh_invoice))
        .routes(routes!(actions::finalize_invoice))
        .routes(routes!(actions::void_invoice))
        .routes(routes!(actions::mark_invoice_uncollectible))
        .routes(routes!(actions::mark_invoice_paid))
        .routes(routes!(actions::record_invoice_payment))
        .routes(routes!(actions::reverse_invoice_payment))
}
