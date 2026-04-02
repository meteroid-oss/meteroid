use crate::api_rest::webhooks::out_model::{
    WebhookOutAddOnEvent, WebhookOutAddOnEventData, WebhookOutCouponEvent,
    WebhookOutCouponEventData, WebhookOutCreditNoteEvent, WebhookOutCreditNoteEventData,
    WebhookOutCustomerEvent, WebhookOutCustomerEventData, WebhookOutEventTypeEnum,
    WebhookOutInvoiceEvent, WebhookOutInvoiceEventData, WebhookOutMetricEvent,
    WebhookOutMetricEventData, WebhookOutPlanEvent, WebhookOutPlanEventData,
    WebhookOutProductEvent, WebhookOutProductEventData, WebhookOutQuoteEvent,
    WebhookOutQuoteEventData, WebhookOutSubscriptionEvent, WebhookOutSubscriptionEventData,
};
use crate::svix::SvixOps;
use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::outbox::to_outbox_events;
use crate::workers::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::pgmq::MessageId;
use error_stack::{Report, ResultExt};
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
                    404 => log::debug!(
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
            OutboxEvent::PlanCreated(event) => {
                let data = WebhookOutPlanEventData::from(*event);
                let event = WebhookOutPlanEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::PlanCreated,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::PlanPublished(event) => {
                let data = WebhookOutPlanEventData::from(*event);
                let event = WebhookOutPlanEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::PlanPublished,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::PlanArchived(event) => {
                let data = WebhookOutPlanEventData::from(*event);
                let event = WebhookOutPlanEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::PlanArchived,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::ProductCreated(event) => {
                let data = WebhookOutProductEventData::from(*event);
                let event = WebhookOutProductEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::ProductCreated,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::ProductUpdated(event) => {
                let data = WebhookOutProductEventData::from(*event);
                let event = WebhookOutProductEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::ProductUpdated,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::ProductArchived(event) => {
                let data = WebhookOutProductEventData::from(*event);
                let event = WebhookOutProductEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::ProductArchived,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::BillableMetricUpdated(event) => {
                let data = WebhookOutMetricEventData::from(*event);
                let event = WebhookOutMetricEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::BillableMetricUpdated,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::BillableMetricArchived(event) => {
                let data = WebhookOutMetricEventData::from(*event);
                let event = WebhookOutMetricEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::BillableMetricArchived,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::CouponCreated(event) => {
                let data = WebhookOutCouponEventData::from(*event);
                let event = WebhookOutCouponEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::CouponCreated,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::CouponUpdated(event) => {
                let data = WebhookOutCouponEventData::from(*event);
                let event = WebhookOutCouponEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::CouponUpdated,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::CouponArchived(event) => {
                let data = WebhookOutCouponEventData::from(*event);
                let event = WebhookOutCouponEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::CouponArchived,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::AddOnCreated(event) => {
                let data = WebhookOutAddOnEventData::from(*event);
                let event = WebhookOutAddOnEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::AddOnCreated,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::AddOnUpdated(event) => {
                let data = WebhookOutAddOnEventData::from(*event);
                let event = WebhookOutAddOnEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::AddOnUpdated,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            OutboxEvent::AddOnArchived(event) => {
                let data = WebhookOutAddOnEventData::from(*event);
                let event = WebhookOutAddOnEvent {
                    id: event_id,
                    event_type: WebhookOutEventTypeEnum::AddOnArchived,
                    data,
                    timestamp,
                };
                Some(event.try_into())
            }
            // Not yet webhook-enabled
            OutboxEvent::CustomerUpdated(_) => None,
            OutboxEvent::InvoiceAccountingPdfGenerated(_) => None,
            OutboxEvent::PaymentTransactionSaved(_) => None,
        }
    }
}

#[async_trait::async_trait]
impl PgmqHandler for WebhookOut {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
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

                    (msg_id, res)
                })
            })
            .collect();

        let results = futures::future::join_all(tasks).await;

        let mut succeeded = Vec::new();
        let mut failed = Vec::new();

        for result in results {
            match result {
                Ok((_, Ok(id))) => succeeded.push(id),
                Ok((msg_id, Err(e))) => failed.push(HandleResult::fail(msg_id, &e)),
                Err(e) => log::error!("Webhook out task panicked: {e:?}"),
            }
        }

        Ok(HandleResult { succeeded, failed })
    }
}
