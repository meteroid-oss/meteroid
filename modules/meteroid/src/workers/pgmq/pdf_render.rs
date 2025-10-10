use crate::services::invoice_rendering::{GenerateResult, PdfRenderingService};
use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::PgmqHandler;
use common_domain::ids::InvoiceId;
use common_domain::pgmq::MessageId;
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
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
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

        let result = self
            .pdf_service
            .generate_pdfs(invoice_ids)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let success_invoice_ids: Vec<InvoiceId> = result
            .iter()
            .filter_map(|x| match x {
                GenerateResult::Success { invoice_id, .. } => Some(*invoice_id),
                _ => None,
            })
            .collect();

        let success_msg_ids = evts
            .iter()
            .filter_map(|(evt, msg_id)| {
                if success_invoice_ids.contains(&evt.invoice_id) {
                    Some(*msg_id)
                } else {
                    None
                }
            })
            .collect();

        Ok(success_msg_ids)
    }
}
