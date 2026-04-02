use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::outbox::to_outbox_events;
use crate::workers::pgmq::processor::{HandleResult, PgmqHandler};
use error_stack::ResultExt;
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
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let msg_id_to_out_evt = to_outbox_events(msgs).await?;

        // TODO optimize by grouping by event type
        let tasks: Vec<_> = msg_id_to_out_evt
            .into_iter()
            .map(|(msg_id, event)| {
                tokio::spawn({
                    let store_clone = self.store.clone();
                    let services_clone = self.services.clone();

                    async move {
                        let result = process_event(&store_clone, &services_clone, event).await;
                        (msg_id, result)
                    }
                })
            })
            .collect();

        let results = futures::future::join_all(tasks).await;

        let mut succeeded = Vec::new();
        let mut failed = Vec::new();

        for result in results {
            match result {
                Ok((msg_id, Ok(()))) => succeeded.push(msg_id),
                Ok((msg_id, Err(err))) => {
                    log::error!(
                        "Failed to handle invoice orchestration message {msg_id:?}: {err:?}"
                    );
                    failed.push((msg_id, format!("{err:?}")));
                }
                Err(join_err) => {
                    log::error!("Task panicked during invoice orchestration: {join_err}");
                }
            }
        }

        Ok(HandleResult {
            succeeded,
            failed,
        })
    }
}

async fn process_event(
    store: &Store,
    services: &Services,
    event: OutboxEvent,
) -> PgmqResult<()> {
    let tenant_id = event.tenant_id();
    match event {
        OutboxEvent::InvoiceFinalized(event) => {
            // request the pdf to be generated
            let evt: StoreResult<PgmqMessageNew> = InvoicePdfRequestEvent::new(
                event.invoice_id,
                false, // we do not want to send the email yet
            )
            .try_into();
            store
                .pgmq_send_batch(
                    PgmqQueue::InvoicePdfRequest,
                    vec![evt.change_context(PgmqError::HandleMessages)?],
                )
                .await
                .change_context(PgmqError::HandleMessages)?;
        }
        OutboxEvent::InvoiceAccountingPdfGenerated(event) => {
            services
                .on_invoice_accounting_pdf_generated(*event, tenant_id)
                .await
                .change_context(PgmqError::HandleMessages)?;
        }
        OutboxEvent::InvoicePaid(event) => {
            services
                .on_invoice_paid(*event, tenant_id)
                .await
                .change_context(PgmqError::HandleMessages)?;
        }
        OutboxEvent::PaymentTransactionSaved(event)
            if event.status == PaymentStatusEnum::Settled =>
        {
            services
                .on_payment_transaction_settled(*event)
                .await
                .change_context(PgmqError::HandleMessages)?;
        }
        OutboxEvent::CreditNoteFinalized(event) => {
            // request the credit note pdf to be generated
            let evt: StoreResult<PgmqMessageNew> =
                CreditNotePdfRequestEvent::new(event.credit_note_id).try_into();
            store
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
            log::warn!("Unhandled outbox event type: {:?}", event);
            return Err(PgmqError::HandleMessages).attach("Invalid event type");
        }
    }

    Ok(())
}
