use crate::api_rest::webhooks::out_model::{
    WebhookOutCreditNoteEvent, WebhookOutCreditNoteEventData, WebhookOutCustomerEvent,
    WebhookOutCustomerEventData, WebhookOutEventTypeEnum, WebhookOutInvoiceEvent,
    WebhookOutInvoiceEventData, WebhookOutMetricEvent, WebhookOutMetricEventData,
    WebhookOutQuoteEvent, WebhookOutQuoteEventData, WebhookOutSubscriptionEvent,
    WebhookOutSubscriptionEventData,
};
use crate::svix::SvixOps;
use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::outbox::to_outbox_events;
use crate::workers::pgmq::processor::PgmqHandler;
use common_domain::pgmq::MessageId;
use error_stack::{Report, ResultExt};
use futures::future::try_join_all;
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqMessage;
use std::sync::Arc;
use svix::api::{MessageIn, Svix};

pub(crate) struct WebhookOut {
    pub svix: Arc<Svix>,
}

impl WebhookOut {
    pub fn new(svix: Arc<Svix>) -> Self {
        Self { svix }
    }

    async fn handle_event(
        msg_id: MessageId,
        event: OutboxEvent,
        svix: Arc<Svix>,
    ) -> Result<MessageId, Report<PgmqError>> {
        let event_id = event.event_id();
        let tenant_id = event.tenant_id();
        let message_in = Self::to_message_in(event);

        if let Some(message_in) = message_in {
            let message_in =
                message_in.map_err(|e| Report::new(PgmqError::HandleMessages).attach(e))?;

            let message_result = svix.create_message(tenant_id, message_in).await;

            if let Err(svix::error::Error::Http(e)) = message_result {
                match e.status.as_u16() {
                    // there is no svix application created for this tenant, yet
                    // it is auto-created once the tenant accesses the webhook portal the first time
                    404 => log::info!(
                        "[svix_404] Skipped webhook {event_id} as the tenant {tenant_id} did not configure webhooks"
                    ),
                    409 => log::info!("[svix_409] Skipped webhook {event_id} as it already exists"),
                    _ => {
                        return Err(svix::error::Error::Http(e))
                            .change_context(PgmqError::HandleMessages);
                    }
                }
            }
        }

        Ok::<MessageId, Report<PgmqError>>(msg_id)
    }

    fn to_message_in(evt: OutboxEvent) -> Option<Result<MessageIn, serde_json::Error>> {
        let event_id = evt.event_id();
        let timestamp = chrono::Utc::now().naive_utc();

        match evt {
            OutboxEvent::CustomerCreated(event) => {
                let data = WebhookOutCustomerEventData::from(*event);
                let event = WebhookOutCustomerEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::CustomerCreated,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::BillableMetricCreated(event) => {
                let data = WebhookOutMetricEventData::from(*event);
                let event = WebhookOutMetricEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::BillableMetricCreated,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::InvoiceCreated(event) => {
                let data = WebhookOutInvoiceEventData::from(*event);
                let event = WebhookOutInvoiceEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::InvoiceCreated,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::InvoiceFinalized(event) => {
                let data = WebhookOutInvoiceEventData::from(*event);
                let event = WebhookOutInvoiceEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::InvoiceFinalized,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::InvoicePaid(event) => {
                let data = WebhookOutInvoiceEventData::from(*event);
                let event = WebhookOutInvoiceEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::InvoicePaid,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::InvoiceVoided(event) => {
                let data = WebhookOutInvoiceEventData::from(*event);
                let event = WebhookOutInvoiceEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::InvoiceVoided,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::SubscriptionCreated(event) => {
                let data = WebhookOutSubscriptionEventData::from(*event);
                let event = WebhookOutSubscriptionEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::SubscriptionCreated,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::QuoteAccepted(event) => {
                let data = WebhookOutQuoteEventData::from(*event);
                let event = WebhookOutQuoteEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::QuoteAccepted,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::QuoteConverted(event) => {
                let data = WebhookOutQuoteEventData::from(*event);
                let event = WebhookOutQuoteEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::QuoteConverted,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::CreditNoteCreated(event) => {
                let data = WebhookOutCreditNoteEventData::from(*event);
                let event = WebhookOutCreditNoteEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::CreditNoteCreated,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::CreditNoteFinalized(event) => {
                let data = WebhookOutCreditNoteEventData::from(*event);
                let event = WebhookOutCreditNoteEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::CreditNoteFinalized,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::CreditNoteVoided(event) => {
                let data = WebhookOutCreditNoteEventData::from(*event);
                let event = WebhookOutCreditNoteEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::CreditNoteVoided,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            // TODO add webhooks
            OutboxEvent::CustomerUpdated(_) => None,
            OutboxEvent::InvoiceAccountingPdfGenerated(_) => None,
            OutboxEvent::PaymentTransactionSaved(_) => None,
        }
    }
}

#[async_trait::async_trait]
impl PgmqHandler for WebhookOut {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
        let msg_id_to_out_evt = to_outbox_events(msgs).await?;

        let tasks: Vec<_> = msg_id_to_out_evt
            .into_iter()
            .map(|(msg_id, event)| {
                let svix = self.svix.clone();
                tokio::spawn(async move {
                    let event_type = event.event_type();
                    let res = Self::handle_event(msg_id, event, svix).await;

                    if let Err(ref e) = res {
                        log::warn!(
                            "Failed to handle webhook_out {} event with msg_id={}: {:?}",
                            event_type,
                            msg_id.0,
                            e
                        );
                    }

                    res
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
