use crate::api_rest::webhooks::out_model::{
    WebhookOutCustomerEventData, WebhookOutEvent, WebhookOutEventData, WebhookOutEventTypeEnum,
    WebhookOutInvoiceEventData, WebhookOutMetricEventData, WebhookOutQuoteEventData,
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
use svix::api::Svix;

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
        let webhook_out = Self::to_webhook_out(event);

        if let Some(webhook_out) = webhook_out {
            let message_result = svix.create_message(tenant_id, webhook_out).await;

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

    fn to_webhook_out(evt: OutboxEvent) -> Option<WebhookOutEvent> {
        let event_id = evt.event_id();
        match evt {
            OutboxEvent::CustomerCreated(event) => {
                let event = WebhookOutCustomerEventData::from(*event);
                Some(WebhookOutEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::CustomerCreated,
                    data: WebhookOutEventData::Customer(event),
                    timestamp: chrono::Utc::now().naive_utc(),
                })
            }
            OutboxEvent::BillableMetricCreated(event) => {
                let event = WebhookOutMetricEventData::from(*event);
                Some(WebhookOutEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::BillableMetricCreated,
                    data: WebhookOutEventData::Metric(event),
                    timestamp: chrono::Utc::now().naive_utc(),
                })
            }
            OutboxEvent::InvoiceCreated(event) => {
                let event = WebhookOutInvoiceEventData::from(*event);
                Some(WebhookOutEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::InvoiceCreated,
                    data: WebhookOutEventData::Invoice(event),
                    timestamp: chrono::Utc::now().naive_utc(),
                })
            }
            OutboxEvent::InvoiceFinalized(event) => {
                let event = WebhookOutInvoiceEventData::from(*event);
                Some(WebhookOutEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::InvoiceFinalized,
                    data: WebhookOutEventData::Invoice(event),
                    timestamp: chrono::Utc::now().naive_utc(),
                })
            }
            OutboxEvent::InvoicePaid(event) => {
                let event = WebhookOutInvoiceEventData::from(*event);
                Some(WebhookOutEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::InvoicePaid,
                    data: WebhookOutEventData::Invoice(event),
                    timestamp: chrono::Utc::now().naive_utc(),
                })
            }
            OutboxEvent::SubscriptionCreated(event) => {
                let event = WebhookOutSubscriptionEventData::from(*event);
                Some(WebhookOutEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::SubscriptionCreated,
                    data: WebhookOutEventData::Subscription(event),
                    timestamp: chrono::Utc::now().naive_utc(),
                })
            }
            OutboxEvent::QuoteAccepted(event) => {
                let event = WebhookOutQuoteEventData::from(*event);
                Some(WebhookOutEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::QuoteAccepted,
                    data: WebhookOutEventData::Quote(event),
                    timestamp: chrono::Utc::now().naive_utc(),
                })
            }
            OutboxEvent::QuoteConverted(event) => {
                let event = WebhookOutQuoteEventData::from(*event);
                Some(WebhookOutEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::QuoteConverted,
                    data: WebhookOutEventData::Quote(event),
                    timestamp: chrono::Utc::now().naive_utc(),
                })
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
