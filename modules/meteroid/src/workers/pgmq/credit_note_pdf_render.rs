use crate::services::credit_note_rendering::{
    CreditNoteGenerateResult, CreditNotePdfRenderingService,
};
use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::PgmqHandler;
use common_domain::ids::CreditNoteId;
use common_domain::pgmq::MessageId;
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
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
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

        let success_credit_note_ids: Vec<CreditNoteId> = result
            .iter()
            .filter_map(|x| match x {
                CreditNoteGenerateResult::Success { credit_note_id, .. } => Some(*credit_note_id),
                _ => None,
            })
            .collect();

        let success_msg_ids = evts
            .iter()
            .filter_map(|(evt, msg_id)| {
                if success_credit_note_ids.contains(&evt.credit_note_id) {
                    Some(*msg_id)
                } else {
                    None
                }
            })
            .collect();

        Ok(success_msg_ids)
    }
}
