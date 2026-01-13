use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::outbox::to_outbox_events;
use crate::workers::pgmq::processor::PgmqHandler;
use common_domain::pgmq::MessageId;
use error_stack::{Report, ResultExt};
use futures::future::try_join_all;
use meteroid_store::domain::PaymentStatusEnum;
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::{
    CreditNotePdfRequestEvent, InvoicePdfRequestEvent, PgmqMessage, PgmqMessageNew, PgmqQueue,
};
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::{Services, Store, StoreResult};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct InvoiceOrchestration {
    store: Arc<Store>,
    services: Arc<Services>,
}

impl InvoiceOrchestration {
    pub fn new(store: Arc<Store>, services: Arc<Services>) -> Self {
        Self { store, services }
    }
}

#[async_trait::async_trait]
impl PgmqHandler for InvoiceOrchestration {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
        let msg_id_to_out_evt = to_outbox_events(msgs).await?;

        // TODO optimize by grouping by event type
        let tasks: Vec<_> = msg_id_to_out_evt
            .into_iter()
            .map(|(msg_id, event)| {
                tokio::spawn({
                    let store_clone = self.store.clone();
                    let services_clone = self.services.clone();

                    async move {
                        let tenant_id = event.tenant_id();
                        match event {
                            OutboxEvent::InvoiceFinalized(event) => {
                                // request the pdf to be generated
                                let evt: StoreResult<PgmqMessageNew> = InvoicePdfRequestEvent::new(
                                    event.invoice_id,
                                    false, // we do not want to send the email yet
                                )
                                .try_into();
                                store_clone
                                    .pgmq_send_batch(
                                        PgmqQueue::InvoicePdfRequest,
                                        vec![evt.change_context(PgmqError::HandleMessages)?],
                                    )
                                    .await
                                    .change_context(PgmqError::HandleMessages)?;
                            }
                            OutboxEvent::InvoiceAccountingPdfGenerated(event) => {
                                services_clone
                                    .on_invoice_accounting_pdf_generated(*event, tenant_id)
                                    .await
                                    .change_context(PgmqError::HandleMessages)?;

                                return Ok(msg_id);
                            }
                            OutboxEvent::InvoicePaid(event) => {
                                services_clone
                                    .on_invoice_paid(*event, tenant_id)
                                    .await
                                    .change_context(PgmqError::HandleMessages)?;

                                return Ok(msg_id);
                            }
                            OutboxEvent::PaymentTransactionSaved(event)
                                if event.status == PaymentStatusEnum::Settled =>
                            {
                                services_clone
                                    .on_payment_transaction_settled(*event)
                                    .await
                                    .change_context(PgmqError::HandleMessages)?;

                                return Ok(msg_id);
                            }
                            OutboxEvent::CreditNoteFinalized(event) => {
                                // request the credit note pdf to be generated
                                let evt: StoreResult<PgmqMessageNew> =
                                    CreditNotePdfRequestEvent::new(event.credit_note_id).try_into();
                                store_clone
                                    .pgmq_send_batch(
                                        PgmqQueue::CreditNotePdfRequest,
                                        vec![evt.change_context(PgmqError::HandleMessages)?],
                                    )
                                    .await
                                    .change_context(PgmqError::HandleMessages)?;
                            }

                            // OutboxEvent::PaymentTransactionSaved(event) if event.status == PaymentStatusEnum::Failed or Cancelled ?
                            // => notify customer if failed, delete draft/checkout invoice, automated payment retry
                            //
                            // OutboxEvent::PaymentReceiptGenerated(event) =>  {
                            //     // we can send the email. Check if the invoice is paid first
                            //
                            // },
                            _ => {
                                return Err(PgmqError::HandleMessages).attach("Invalid event type");
                            }
                        }

                        Ok::<MessageId, Report<PgmqError>>(msg_id)
                    }
                })
            })
            .collect();

        let results = try_join_all(tasks)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let ids: Vec<_> = results.into_iter().filter_map(Result::ok).collect();

        Ok(ids)
    }
}
