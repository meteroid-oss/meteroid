use crate::pgmq::PgmqResult;
use crate::pgmq::error::PgmqError;
use crate::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::ids::{ConnectorId, StoredDocumentId};
use error_stack::{Report, ResultExt};
use meteroid_app::services::storage::{self, ObjectStoreService};
use meteroid_store::domain::pgmq::{PgmqMessage, WebhookInProcessEvent};
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::{Services, Store};
use std::sync::Arc;

/// Async processor for inbound webhooks. The pgmq message only carries
/// `{ webhook_in_event_id, tenant_id }`; the raw body is re-read from object
/// storage via the `webhook_in_event` row.
pub struct WebhookIn {
    services: Arc<Services>,
    store: Arc<Store>,
    object_store: Arc<dyn ObjectStoreService>,
}

impl WebhookIn {
    pub(crate) fn new(
        services: Arc<Services>,
        store: Arc<Store>,
        object_store: Arc<dyn ObjectStoreService>,
    ) -> Self {
        Self {
            services,
            store,
            object_store,
        }
    }

    #[tracing::instrument(skip_all, fields(
        tenant_id = %ev.tenant_id,
        webhook_in_event_id = %ev.webhook_in_event_id,
    ))]
    async fn process_incoming_webhook(
        &self,
        ev: &WebhookInProcessEvent,
    ) -> Result<(), Report<PgmqError>> {
        let result = self.process_event(ev).await;
        if let Err(err) = &result {
            log::error!(
                "Failed to process inbound webhook {}: {:?}",
                ev.webhook_in_event_id,
                err
            );
            // Best-effort: record the failure on the audit row. pgmq
            // retries (and eventually dead-letters) the message itself.
            let _ = self
                .services
                .mark_webhook_in_failed(ev.webhook_in_event_id, format!("{err:?}"))
                .await;
        }
        result
    }

    async fn process_event(&self, ev: &WebhookInProcessEvent) -> Result<(), Report<PgmqError>> {
        // The audit row holds the connector id and the object-store key.
        let row = self
            .services
            .get_webhook_in_event(ev.webhook_in_event_id)
            .await
            .change_context(PgmqError::HandleMessages)?;

        if row.processed_at.is_some() {
            // Already handled (e.g. a redelivery after a late ack). No-op.
            return Ok(());
        }

        let connector = self
            .store
            .get_connector_with_data(ConnectorId::from(row.provider_config_id), ev.tenant_id)
            .await
            .change_context(PgmqError::HandleMessages)?;

        // Re-read the verified raw body from object storage, addressed by its registry record.
        let bytes = storage::read(
            self.object_store.as_ref(),
            &self.store,
            StoredDocumentId::from(row.id),
        )
        .await
        .change_context(PgmqError::HandleMessages)?;

        let connector_impl =
            meteroid_store::adapters::payment::initialize_payment_connector(&connector)
                .change_context(PgmqError::HandleMessages)?;

        // Signature was already verified at ingest, so empty headers are fine here.
        let events = connector_impl
            .parse_events(bytes.as_ref(), &axum::http::HeaderMap::new())
            .change_context(PgmqError::HandleMessages)?;

        let mut discarded: Vec<String> = Vec::new();
        for event in events {
            let event_id = event.provider_event_id.clone();
            let result = meteroid_app::payment_webhooks::event_handler::handle_normalized_event(
                event,
                &connector,
                connector_impl.as_ref(),
                (*self.store).clone(),
                self.services.as_ref(),
            )
            .await;

            if let Err(err) = result {
                if err.current_context().is_transient() {
                    // A DB write or provider fetch failed transiently; propagate
                    // so pgmq retries this message.
                    return Err(err).change_context(PgmqError::HandleMessages);
                }
                // Not ours / permanently un-processable (bad metadata, unknown
                // event). Ack it so a poison event never blocks the queue, and
                // record why on the audit row for forensics.
                log::warn!("Acking non-retryable webhook event {event_id}: {err:?}");
                discarded.push(format!("discarded non-retryable event {event_id}: {err:?}"));
            }
        }

        self.services
            .mark_webhook_in_processed(
                ev.webhook_in_event_id,
                (!discarded.is_empty()).then(|| discarded.join("\n")),
            )
            .await
            .change_context(PgmqError::HandleMessages)?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl PgmqHandler for WebhookIn {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let mut succeeded = Vec::new();
        let mut failed = Vec::new();

        for msg in msgs {
            let event: WebhookInProcessEvent = match msg.try_into() {
                Ok(event) => event,
                Err(err) => {
                    log::error!("Failed to decode webhook_in message: {err:?}");
                    failed.push(HandleResult::fail(msg.msg_id, &err));
                    continue;
                }
            };

            match self.process_incoming_webhook(&event).await {
                Ok(()) => succeeded.push(msg.msg_id),
                Err(err) => failed.push(HandleResult::fail(msg.msg_id, &err)),
            }
        }

        Ok(HandleResult { succeeded, failed })
    }
}
