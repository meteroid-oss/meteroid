use crate::pgmq::PgmqResult;
use crate::pgmq::error::PgmqError;
use crate::pgmq::outbox::decode_each;
use crate::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::ids::CreditNoteId;
use error_stack::ResultExt;
use meteroid_app::services::credit_note_rendering::{
    CreditNoteGenerateResult, CreditNotePdfRenderingService,
};
use meteroid_store::domain::customer_emails::{EmailContext, EmailKind};
use meteroid_store::domain::pgmq::{CreditNotePdfRequestEvent, PgmqMessage};
use meteroid_store::repositories::CustomerEmailsInterface;
use meteroid_store::{Store, StoreResult};
use std::sync::Arc;

pub(crate) struct CreditNotePdfRender {
    store: Arc<Store>,
    pdf_service: Arc<CreditNotePdfRenderingService>,
}

impl CreditNotePdfRender {
    pub fn new(store: Arc<Store>, pdf_service: Arc<CreditNotePdfRenderingService>) -> Self {
        Self { store, pdf_service }
    }
}

#[async_trait::async_trait]
impl PgmqHandler for CreditNotePdfRender {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let decoded = decode_each(msgs, |msg| {
            let evt: StoreResult<CreditNotePdfRequestEvent> = msg.try_into();
            evt.map(Some)
        });

        let credit_note_ids = decoded
            .events
            .iter()
            .map(|(_, evt)| evt.credit_note_id)
            .collect();

        let result = self
            .pdf_service
            .generate_pdfs(credit_note_ids)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let mut success_credit_note_ids: Vec<CreditNoteId> = Vec::new();
        let mut failed_credit_note_ids: Vec<(CreditNoteId, String)> = Vec::new();

        for r in &result {
            match r {
                CreditNoteGenerateResult::Success {
                    credit_note_id,
                    tenant_id,
                    ..
                } => {
                    // The stored PDF is what the email attaches; the key collapses re-runs.
                    if let Err(e) = self
                        .store
                        .enqueue_customer_email(
                            *tenant_id,
                            EmailKind::CreditNoteIssued,
                            EmailContext::CreditNote {
                                credit_note_id: *credit_note_id,
                            },
                            format!("credit_note_issued:{credit_note_id}"),
                        )
                        .await
                    {
                        log::warn!(
                            "Failed to enqueue credit_note_issued for {credit_note_id}: {e:?}"
                        );
                        failed_credit_note_ids.push((*credit_note_id, format!("{e:?}")));
                        continue;
                    }
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
        let mut failed = decoded.failed;

        for (msg_id, evt) in &decoded.events {
            if success_credit_note_ids.contains(&evt.credit_note_id) {
                succeeded.push(*msg_id);
            } else if let Some((_, error)) = failed_credit_note_ids
                .iter()
                .find(|(id, _)| *id == evt.credit_note_id)
            {
                failed.push((*msg_id, error.clone()));
            } else {
                failed.push((
                    *msg_id,
                    format!("Credit note {} was not rendered", evt.credit_note_id),
                ));
            }
        }

        Ok(HandleResult { succeeded, failed })
    }
}
