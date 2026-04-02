use crate::services::invoice_rendering::{GenerateResult, PdfRenderingService};
use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::ids::InvoiceId;
use error_stack::ResultExt;
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
        let mut evts = vec![];

        for msg in msgs {
            let evt: StoreResult<InvoicePdfRequestEvent> = msg.try_into();

            match evt {
                Ok(evt) => {
                    evts.push((evt, msg.msg_id));
                }
                Err(e) => {
                    log::warn!("failed to convert message to InvoicePdfRequestEvent: {e:?}");
                }
            }
        }

        let invoice_ids = evts.iter().map(|(evt, _)| evt.invoice_id).collect();

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
                    log::warn!("Failed to generate pdf for invoice {invoice_id}: {error}");
                    failed_invoice_ids.push((*invoice_id, error.clone()));
                }
            }
        }

        let mut succeeded = Vec::new();
        let mut failed = Vec::new();

        for (evt, msg_id) in &evts {
            if let Some((_, error)) = failed_invoice_ids
                .iter()
                .find(|(id, _)| *id == evt.invoice_id)
            {
                failed.push((*msg_id, error.clone()));
            } else if success_invoice_ids.contains(&evt.invoice_id) {
                succeeded.push(*msg_id);
            }
        }

        Ok(HandleResult {
            succeeded,
            failed,
        })
    }
}
