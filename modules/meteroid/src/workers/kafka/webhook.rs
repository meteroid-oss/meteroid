use crate::workers::kafka::outbox::{parse_outbox_event, EventType, OutboxEvent};
use crate::workers::kafka::processor::MessageHandler;
use async_trait::async_trait;
use chrono::SecondsFormat;
use error_stack::Report;
use meteroid_store::domain::enums::WebhookOutEventTypeEnum;
use meteroid_store::domain::outbox_event::CustomerCreatedEvent;
use meteroid_store::domain::webhooks::{
    WebhookOutCreateMessageResult, WebhookOutMessageNew, WebhookOutMessagePayload,
};
use meteroid_store::domain::{Address, ShippingAddress};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::webhooks::WebhooksInterface;
use meteroid_store::Store;
use o2o::o2o;
use serde::Serialize;
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
        if let Some(event) = parse_outbox_event(message) {
            log::info!("Processing message: {:?}", event);

            let tenant_id = event.tenant_id;

            let wh: Option<WebhookOutMessageNew> = event.try_into()?;

            if let Some(wh) = wh {
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
                log::debug!("Skipping outbox message");
            }
        } else {
            log::debug!("Skipping message");
        }

        Ok(())
    }
}

#[derive(Debug, Serialize, o2o)]
#[from_owned(CustomerCreatedEvent)]
pub struct CustomerCreated {
    #[map(local_id)]
    pub id: String,
    pub name: String,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub currency: String,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
}

impl TryInto<Option<WebhookOutMessageNew>> for OutboxEvent {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<Option<WebhookOutMessageNew>, Self::Error> {
        let (event_type, payload) = match self.event_type {
            EventType::CustomerCreated(event) => {
                let event_type = WebhookOutEventTypeEnum::CustomerCreated;

                let event = CustomerCreated::from(*event);
                let payload = serde_json::to_value(event).map_err(|e| {
                    Report::from(StoreError::SerdeError(
                        "Failed to serialize payload".to_string(),
                        e,
                    ))
                })?;

                (event_type, WebhookOutMessagePayload::Customer(payload))
            }
            _ => return Ok(None),
        };

        let created_at = self
            .event_timestamp
            .to_rfc3339_opts(SecondsFormat::Millis, true);

        let webhook = WebhookOutMessageNew {
            id: self.id,
            event_type,
            payload,
            created_at,
        };

        Ok(Some(webhook))
    }
}
