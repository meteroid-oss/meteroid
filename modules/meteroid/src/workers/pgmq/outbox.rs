use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::PgmqHandler;
use async_trait::async_trait;
use common_domain::pgmq::{Headers, Message, MessageId};
use error_stack::{Report, ResultExt, report};
use meteroid_store::domain::outbox_event::{EventType, OutboxEvent, OutboxPgmqHeaders};
use meteroid_store::domain::pgmq::{
    InvoicePdfRequestEvent, PgmqMessage, PgmqMessageNew, PgmqQueue,
};
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::{Store, StoreResult};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Dispatches to consumer queues. This allows to mimic kafka consumer groups.
/// Note:
/// The message column is not copied to the consumer queue.
/// The original message id is set in the headers so consumer queue reader can fetch the messages from the archived outbox queue by ids.
pub struct PgmqOutboxDispatch {
    pub(crate) store: Arc<Store>,
}

impl PgmqOutboxDispatch {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    pub(crate) async fn handle_webhook_out(&self, msgs: &[PgmqMessage]) -> PgmqResult<()> {
        let wh_messages = msgs
            .iter()
            .filter_map(|x| {
                let headers: Headers = DispatchHeaders {
                    outbox_msg_id: x.msg_id,
                }
                .try_into()
                .ok()?;

                Some(PgmqMessageNew {
                    message: Message(None),
                    headers,
                })
            })
            .collect();

        self.store
            .pgmq_send_batch(PgmqQueue::WebhookOut, wh_messages)
            .await
            .change_context(PgmqError::HandleMessages)
    }

    pub(crate) async fn handle_invoice_pdf_requests(&self, msgs: &[PgmqMessage]) -> PgmqResult<()> {
        let mut pdf_requests = vec![];

        for msg in msgs {
            let out_headers: StoreResult<OutboxPgmqHeaders> = (&msg.headers).try_into();
            if let Ok(out_headers) = out_headers {
                if let EventType::InvoiceFinalized = &out_headers.event_type {
                    if let Ok(OutboxEvent::InvoiceFinalized(evt)) = msg.try_into() {
                        if let Ok(msg_new) = InvoicePdfRequestEvent::new(evt.invoice_id).try_into()
                        {
                            pdf_requests.push(msg_new);
                        }
                    }
                }
            }
        }

        if !pdf_requests.is_empty() {
            self.store
                .pgmq_send_batch(PgmqQueue::InvoicePdfRequest, pdf_requests)
                .await
                .change_context(PgmqError::HandleMessages)?;
        }

        Ok(())
    }
}

#[async_trait]
impl PgmqHandler for PgmqOutboxDispatch {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
        let ids = msgs.iter().map(|x| x.msg_id).collect::<Vec<_>>();

        // Run the functions in parallel
        let (webhook_result, pdf_result) = tokio::join!(
            self.handle_webhook_out(msgs),
            self.handle_invoice_pdf_requests(msgs)
        );

        // Check results
        webhook_result?;
        pdf_result?;

        Ok(ids)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DispatchHeaders {
    pub outbox_msg_id: MessageId,
}

impl TryInto<Headers> for DispatchHeaders {
    type Error = Report<PgmqError>;

    fn try_into(self) -> Result<Headers, Self::Error> {
        serde_json::to_value(&self)
            .map(Headers::some)
            .change_context(PgmqError::HandleMessages)
    }
}

impl TryInto<DispatchHeaders> for &PgmqMessage {
    type Error = Report<PgmqError>;

    fn try_into(self) -> Result<DispatchHeaders, Self::Error> {
        let headers = self.headers.0.as_ref().ok_or(PgmqError::EmptyHeaders)?;

        DispatchHeaders::deserialize(headers.clone()).map_err(|e| report!(PgmqError::Serde(e)))
    }
}

/// Proxy Handler that lists the archived outbox messages, passes them to the underlying handler,
/// then matches the results to the original messages and returns the original message ids so they can be acked.
pub(crate) struct PgmqOutboxProxy {
    pub(crate) store: Arc<Store>,
    pub(crate) underlying: Arc<dyn PgmqHandler>,
}

impl PgmqOutboxProxy {
    pub fn new(store: Arc<Store>, underlying: Arc<dyn PgmqHandler>) -> Self {
        Self { store, underlying }
    }
}

#[async_trait::async_trait]
impl PgmqHandler for PgmqOutboxProxy {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
        if msgs.is_empty() {
            return Ok(vec![]);
        }

        let msg_ids = msgs.iter().try_fold(Vec::new(), |mut acc, msg| {
            let DispatchHeaders { outbox_msg_id } = msg.try_into()?;
            acc.push((msg.msg_id, outbox_msg_id));
            Ok::<Vec<_>, Report<PgmqError>>(acc)
        })?;

        let (_, outbox_msg_ids): (Vec<_>, Vec<_>) = msg_ids.iter().cloned().unzip();

        let outbox_archived: Vec<PgmqMessage> = self
            .store
            .pgmq_list_archived(PgmqQueue::OutboxEvent, outbox_msg_ids)
            .await
            .change_context(PgmqError::ListArchived)?;

        let succeeded_archived = self.underlying.handle(&outbox_archived).await?;

        let succeeded_original_ids = msg_ids
            .iter()
            .filter_map(|(orig, out)| {
                if succeeded_archived.contains(out) {
                    Some(*orig)
                } else {
                    None
                }
            })
            .collect();

        Ok(succeeded_original_ids)
    }
}

pub(crate) async fn to_outbox_events(
    msgs: &[PgmqMessage],
) -> PgmqResult<Vec<(MessageId, OutboxEvent)>> {
    msgs.iter().try_fold(vec![], |mut acc, msg| {
        let outbox_event: StoreResult<OutboxEvent> = msg.try_into();
        let outbox_event = outbox_event.change_context(PgmqError::ListArchived)?;

        acc.push((msg.msg_id, outbox_event));

        Ok::<Vec<_>, Report<PgmqError>>(acc)
    })
}
