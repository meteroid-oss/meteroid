use crate::pgmq::PgmqResult;
use crate::pgmq::error::PgmqError;
use crate::pgmq::outbox::decode_each;
use crate::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::ids::InvoiceId;
use error_stack::ResultExt;
use meteroid_app::services::invoice_rendering::{GenerateResult, PdfRenderingService};
use meteroid_store::StoreResult;
use meteroid_store::domain::pgmq::{InvoicePdfRequestEvent, PgmqMessage};
use std::sync::Arc;

pub(crate) struct PdfRender {
    pdf_service: Arc<PdfRenderingService>,
}

impl PdfRender {
    pub fn new(pdf_service: Arc<PdfRenderingService>) -> Self {
        Self { pdf_service }
    }
}

#[async_trait::async_trait]
impl PgmqHandler for PdfRender {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let decoded = decode_each(msgs, |msg| {
            let evt: StoreResult<InvoicePdfRequestEvent> = msg.try_into();
            evt.map(Some)
        });

        let invoice_ids = decoded
            .events
            .iter()
            .map(|(_, evt)| evt.invoice_id)
            .collect();

        let results = self
            .pdf_service
            .generate_pdfs(invoice_ids)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let mut success_invoice_ids: Vec<InvoiceId> = Vec::new();
        let mut failed_invoice_ids: Vec<(InvoiceId, String)> = Vec::new();

        for r in &results {
            match r {
                GenerateResult::Success { invoice_id, .. } => {
                    success_invoice_ids.push(*invoice_id);
                }
                GenerateResult::Failure { invoice_id, error } => {
                    let tenant_id = decoded
                        .events
                        .iter()
                        .find(|(_, evt)| evt.invoice_id == *invoice_id)
                        .and_then(|(_, evt)| evt.tenant_id)
                        .map(tracing::field::display);
                    tracing::warn!(
                        tenant_id,
                        invoice_id = %invoice_id,
                        "Failed to generate pdf for invoice {invoice_id}: {error}"
                    );
                    failed_invoice_ids.push((*invoice_id, error.clone()));
                }
            }
        }

        let mut succeeded = Vec::new();
        let mut failed = decoded.failed;

        for (msg_id, evt) in &decoded.events {
            if let Some((_, error)) = failed_invoice_ids
                .iter()
                .find(|(id, _)| *id == evt.invoice_id)
            {
                failed.push((*msg_id, error.clone()));
            } else if success_invoice_ids.contains(&evt.invoice_id) {
                succeeded.push(*msg_id);
            } else {
                failed.push((
                    *msg_id,
                    format!("Invoice {} was not rendered", evt.invoice_id),
                ));
            }
        }

        Ok(HandleResult { succeeded, failed })
    }
}
