use crate::workers::kafka::outbox::from_kafka_message;
use crate::workers::kafka::processor::MessageHandler;
use crate::workers::webhook_out::to_webhook_out;
use async_trait::async_trait;
use meteroid_store::Store;
use meteroid_store::domain::webhooks::{WebhookOutCreateMessageResult, WebhookOutMessageNew};
use meteroid_store::repositories::webhooks::WebhooksInterface;
use std::sync::Arc;

pub struct WebhookHandler {
    store: Arc<Store>,
}

impl WebhookHandler {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl MessageHandler for WebhookHandler {
    async fn handle(
        &self,
        message: &rdkafka::message::BorrowedMessage<'_>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(event) = from_kafka_message(message) {
            log::info!("Processing message: {:?}", event);

            let tenant_id = event.tenant_id();

            let wh: WebhookOutMessageNew = to_webhook_out(event)?;

            let webhook_type = wh.event_type.to_string();
            let event_id = wh.id.clone();

            let result = self.store.insert_webhook_message_out(tenant_id, wh).await?;

            match result {
                WebhookOutCreateMessageResult::Created(_) => {
                    log::info!("Sent {} webhook with id {}", webhook_type, event_id)
                }
                WebhookOutCreateMessageResult::Conflict => {
                    log::warn!(
                        "Skipped {} webhook with id {} as it already exists",
                        webhook_type,
                        event_id
                    )
                }
                WebhookOutCreateMessageResult::NotFound => {
                    log::warn!(
                        "Skipped {} webhook with id {} as the webhooks seem to not be configured for tenant {}",
                        webhook_type,
                        event_id,
                        tenant_id
                    )
                }
                WebhookOutCreateMessageResult::SvixNotConfigured => {
                    log::warn!(
                        "Skipped {} webhook with id {} as svix client not configured",
                        webhook_type,
                        event_id
                    )
                }
            }
        } else {
            log::debug!("Skipping message");
        }

        Ok(())
    }
}
