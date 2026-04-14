use crate::api_rest::webhooks::out_model::{
    WebhookOutAddOnEvent, WebhookOutAddOnEventData, WebhookOutCouponEvent,
    WebhookOutCouponEventData, WebhookOutCreditNoteEvent, WebhookOutCreditNoteEventData,
    WebhookOutCustomerEvent, WebhookOutCustomerEventData, WebhookOutEventTypeEnum,
    WebhookOutInvoiceEvent, WebhookOutInvoiceEventData, WebhookOutMetricEvent,
    WebhookOutMetricEventData, WebhookOutPlanEvent, WebhookOutPlanEventData,
    WebhookOutProductEvent, WebhookOutProductEventData, WebhookOutQuoteEvent,
    WebhookOutQuoteEventData, WebhookOutSubscriptionEvent, WebhookOutSubscriptionEventData,
};
use crate::services::svix_cache::{
    EndpointConfig, LockOutcome, SvixEndpointCache, build_endpoint_config,
};
use crate::svix::SvixOps;
use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::outbox::to_outbox_events;
use crate::workers::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::ids::TenantId;
use common_domain::pgmq::MessageId;
use error_stack::{Report, ResultExt};
use futures::future::{join_all, try_join_all};
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqMessage;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use svix::api::MessageIn;

pub(crate) struct WebhookOut {
    pub svix: Arc<dyn SvixOps>,
    pub endpoint_cache: Arc<dyn SvixEndpointCache>,
}

impl WebhookOut {
    pub fn new(svix: Arc<dyn SvixOps>, endpoint_cache: Arc<dyn SvixEndpointCache>) -> Self {
        Self {
            svix,
            endpoint_cache,
        }
    }

    /// Fail-open on any underlying error (returns `send_all`).
    async fn resolve_endpoint_config(
        svix: &Arc<dyn SvixOps>,
        cache: &Arc<dyn SvixEndpointCache>,
        tenant_id: &TenantId,
    ) -> EndpointConfig {
        if let Some(c) = cache.get(tenant_id).await {
            return c;
        }

        match cache.try_acquire_lock(tenant_id).await {
            LockOutcome::CachedByOther(c) => c,
            LockOutcome::Acquired => {
                let result = Self::load_and_store(svix, cache, tenant_id).await;
                cache.release_lock(tenant_id).await;
                result
            }
            LockOutcome::TimedOut | LockOutcome::NoCache => {
                Self::load_and_store(svix, cache, tenant_id).await
            }
        }
    }

    async fn load_and_store(
        svix: &Arc<dyn SvixOps>,
        cache: &Arc<dyn SvixEndpointCache>,
        tenant_id: &TenantId,
    ) -> EndpointConfig {
        match svix.list_endpoints(*tenant_id).await {
            Ok(endpoints) => {
                let config = build_endpoint_config(&endpoints);
                cache.store(tenant_id, &config).await;
                config
            }
            Err(svix::error::Error::Http(e)) if e.status.as_u16() == 404 => {
                let config = EndpointConfig::empty();
                cache.store(tenant_id, &config).await;
                config
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to list Svix endpoints for tenant {tenant_id}: {e:?}. Fail-open."
                );
                EndpointConfig::send_all()
            }
        }
    }

    async fn handle_event(
        msg_id: MessageId,
        event: OutboxEvent,
        svix: Arc<dyn SvixOps>,
        endpoint_cache: Arc<dyn SvixEndpointCache>,
        endpoint_config: EndpointConfig,
    ) -> Result<MessageId, Report<PgmqError>> {
        let event_id = event.event_id();
        let tenant_id = event.tenant_id();
        let message_in = Self::to_message_in(event);

        if let Some(message_in) = message_in {
            let message_in =
                message_in.map_err(|e| Report::new(PgmqError::HandleMessages).attach(e))?;

            if !endpoint_config.should_send(&message_in.event_type) {
                tracing::debug!(
                    "[svix_cache] Skipped webhook {event_id} for tenant {tenant_id}: no active endpoint for {}",
                    message_in.event_type
                );
                return Ok(msg_id);
            }

            let message_result = svix.create_message(tenant_id, message_in).await;

            if let Err(svix::error::Error::Http(e)) = message_result {
                match e.status.as_u16() {
                    404 => {
                        endpoint_cache.invalidate(&tenant_id).await;
                        tracing::debug!(
                            "[svix_404] Skipped webhook {event_id} as the tenant {tenant_id} did not configure webhooks"
                        );
                    }
                    409 => {
                        tracing::info!("[svix_409] Skipped webhook {event_id} as it already exists")
                    }
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

        // Resolve once per unique tenant — collapses N-per-tenant bursts (e.g. 00:00 invoicing) to 1 roundtrip.
        let unique_tenants: HashSet<TenantId> = msg_id_to_out_evt
            .iter()
            .map(|(_, e)| e.tenant_id())
            .collect();

        let resolved: Vec<(TenantId, EndpointConfig)> =
            join_all(unique_tenants.into_iter().map(|tid| {
                let svix = self.svix.clone();
                let cache = self.endpoint_cache.clone();
                async move {
                    let config = Self::resolve_endpoint_config(&svix, &cache, &tid).await;
                    (tid, config)
                }
            }))
            .await;

        let configs: HashMap<TenantId, EndpointConfig> = resolved.into_iter().collect();

        let tasks: Vec<_> = msg_id_to_out_evt
            .into_iter()
            .map(|(msg_id, event)| {
                let svix = self.svix.clone();
                let endpoint_cache = self.endpoint_cache.clone();
                let endpoint_config = configs
                    .get(&event.tenant_id())
                    .cloned()
                    .unwrap_or_else(EndpointConfig::send_all);
                tokio::spawn(async move {
                    let event_type = event.event_type();
                    let res =
                        Self::handle_event(msg_id, event, svix, endpoint_cache, endpoint_config)
                            .await;

                    if let Err(ref e) = res {
                        tracing::warn!(
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

        let results = try_join_all(tasks)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let mut succeeded = Vec::new();
        let mut failed = Vec::new();

        for (msg_id, result) in results {
            match result {
                Ok(id) => succeeded.push(id),
                Err(e) => failed.push(HandleResult::fail(msg_id, &e)),
            }
        }

        Ok(HandleResult { succeeded, failed })
    }
}
