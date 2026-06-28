use crate::adapters::stripe::Stripe;
use crate::adapters::types::{ParsedRequest, WebhookAdapter};
use crate::services::storage::{ObjectStoreService, Prefix};
use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::ids::{ConnectorId, StoredDocumentId};
use error_stack::{Report, ResultExt};
use meteroid_store::domain::enums::ConnectorProviderEnum;
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
    stripe_adapter: Arc<Stripe>,
}

impl WebhookIn {
    pub(crate) fn new(
        services: Arc<Services>,
        store: Arc<Store>,
        object_store: Arc<dyn ObjectStoreService>,
        stripe_adapter: Arc<Stripe>,
    ) -> Self {
        Self {
            services,
            store,
            object_store,
            stripe_adapter,
        }
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

        let adapter = match connector.provider {
            ConnectorProviderEnum::Stripe => self.stripe_adapter.clone(),
            other => {
                return Err(Report::new(PgmqError::HandleMessages)
                    .attach(format!("Unsupported inbound webhook provider: {other:?}")));
            }
        };

        // Re-read the verified raw body from object storage. The row id is the
        // object-store uid, and the prefix is rebuilt from the connector alias.
        let prefix = Prefix::WebhookArchive {
            connection_alias: connector.alias.clone(),
            tenant_id: ev.tenant_id,
        };

        let bytes = self
            .object_store
            .retrieve(StoredDocumentId::from(row.id), prefix)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let json_body: serde_json::Value =
            serde_json::from_slice(bytes.as_ref()).change_context(PgmqError::HandleMessages)?;

        // Signature was already verified at ingest, so headers are not needed here.
        let parsed = ParsedRequest {
            method: axum::http::Method::POST,
            headers: axum::http::header::HeaderMap::new(),
            raw_body: bytes,
            json_body,
            query_params: None,
        };

        adapter
            .process_webhook_event(&parsed, &connector, (*self.store).clone())
            .await
            .change_context(PgmqError::HandleMessages)?;

        self.services
            .mark_webhook_in_processed(ev.webhook_in_event_id)
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

            match self.process_event(&event).await {
                Ok(()) => succeeded.push(msg.msg_id),
                Err(err) => {
                    log::error!(
                        "Failed to process inbound webhook {}: {:?}",
                        event.webhook_in_event_id,
                        err
                    );
                    // Best-effort: record the failure on the audit row. pgmq
                    // retries (and eventually dead-letters) the message itself.
                    let _ = self
                        .services
                        .mark_webhook_in_failed(event.webhook_in_event_id, format!("{err:?}"))
                        .await;
                    failed.push(HandleResult::fail(msg.msg_id, &err));
                }
            }
        }

        Ok(HandleResult { succeeded, failed })
    }
}
