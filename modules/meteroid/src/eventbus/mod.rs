use crate::api::services::utils::uuid_gen;
use std::sync::Arc;
use uuid::Uuid;

pub mod memory;
pub mod webhook_handler;

#[derive(thiserror::Error, Debug, Clone)]
pub enum EventBusError {
    #[error("Failed to publish event")]
    PublishFailed,
    #[error("Failed to handle event: {0}")]
    EventHandlerFailed(String),
}

#[async_trait::async_trait]
pub trait EventHandler<E>: Send + Sync {
    async fn handle(&self, event: E) -> Result<(), EventBusError>;
}

#[async_trait::async_trait]
pub trait EventBus<E>: Send + Sync {
    async fn subscribe(&self, handler: Arc<dyn EventHandler<E>>);
    async fn publish(&self, event: E) -> Result<(), EventBusError>;
}

#[derive(Debug, Clone)]
pub struct Event {
    pub event_id: Uuid,
    pub event_timestamp: chrono::DateTime<chrono::Utc>,
    pub event_data: EventData,
}

impl Event {
    pub fn new(event_data: EventData) -> Self {
        Self {
            event_id: uuid_gen::v7(),
            event_timestamp: chrono::Utc::now(),
            event_data,
        }
    }

    pub fn customer_created(customer_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(EventData::CustomerCreated(TenantEventDataDetails {
            tenant_id,
            entity_id: customer_id,
        }))
    }

    pub fn subscription_created(subscription_id: Uuid, tenant_id: Uuid) -> Self {
        Self::new(EventData::SubscriptionCreated(TenantEventDataDetails {
            tenant_id,
            entity_id: subscription_id,
        }))
    }
}

#[derive(Debug, Clone)]
pub enum EventData {
    OrganizationCreated(EventDataDetails),
    TenantCreated(TenantEventDataDetails),
    CustomerCreated(TenantEventDataDetails),
    SubscriptionCreated(TenantEventDataDetails),
    InvoiceCreated(TenantEventDataDetails),
    InvoiceFinalized(TenantEventDataDetails),
}

#[derive(Debug, Clone)]
pub struct EventDataDetails {
    pub entity_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct TenantEventDataDetails {
    pub tenant_id: Uuid,
    pub entity_id: Uuid,
}
