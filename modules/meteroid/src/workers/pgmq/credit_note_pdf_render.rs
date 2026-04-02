use crate::services::credit_note_rendering::{
    CreditNoteGenerateResult, CreditNotePdfRenderingService,
};
use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::ids::CreditNoteId;
use error_stack::ResultExt;
use meteroid_store::StoreResult;
use meteroid_store::domain::pgmq::{CreditNotePdfRequestEvent, PgmqMessage};
use std::sync::Arc;

pub(crate) struct CreditNotePdfRender {
    pdf_service: Arc<CreditNotePdfRenderingService>,
}

impl CreditNotePdfRender {
    pub fn new(pdf_service: Arc<CreditNotePdfRenderingService>) -> Self {
        Self { pdf_service }
    }
}

#[async_trait::async_trait]
impl PgmqHandler for CreditNotePdfRender {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let mut evts = vec![];

        for msg in msgs {
            let evt: StoreResult<CreditNotePdfRequestEvent> = msg.try_into();

            match evt {
                Ok(evt) => {
                    evts.push((evt, msg.msg_id));
                }
                Err(e) => {
                    log::warn!("failed to convert message to CreditNotePdfRequestEvent: {e:?}");
                }
            }
        }

        let credit_note_ids = evts.iter().map(|(evt, _)| evt.credit_note_id).collect();

        let result = self
            .pdf_service
            .generate_pdfs(credit_note_ids)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let mut success_credit_note_ids: Vec<CreditNoteId> = Vec::new();
        let mut failed_credit_note_ids: Vec<(CreditNoteId, String)> = Vec::new();

        for r in &result {
            match r {
                CreditNoteGenerateResult::Success { credit_note_id, .. } => {
                    success_credit_note_ids.push(*credit_note_id);
                }
                CreditNoteGenerateResult::Failure {
                    credit_note_id,
                    error,
                } => {
                    log::warn!("Failed to generate pdf for credit note {credit_note_id}: {error}");
                    failed_credit_note_ids.push((*credit_note_id, error.clone()));
                }
            }
        }

        let mut succeeded = Vec::new();
        let mut failed = Vec::new();

        for (evt, msg_id) in &evts {
            if success_credit_note_ids.contains(&evt.credit_note_id) {
                succeeded.push(*msg_id);
            } else if let Some((_, error)) = failed_credit_note_ids
                .iter()
                .find(|(id, _)| *id == evt.credit_note_id)
            {
                failed.push((*msg_id, error.clone()));
            }
        }

        Ok(HandleResult { succeeded, failed })
    }
}
