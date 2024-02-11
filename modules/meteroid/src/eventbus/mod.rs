use std::sync::Arc;
use uuid::Uuid;

pub mod memory;

#[derive(thiserror::Error, Debug, Clone)]
pub enum EventBusError {
    #[error("Failed to publish event")]
    PublishFailed,
    #[error("Failed to handle event {0}: {1}")]
    EventHandlerFailed(Uuid, String),
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

#[derive(Debug)]
struct Event {
    pub event_id: Uuid,
    pub event_timestamp: chrono::DateTime<chrono::Utc>,
    pub event_data: EventData,
}

#[derive(Debug)]
enum EventData {
    OrganizationCreated(EventDataDetails),
    TenantCreated(TenantEventDataDetails),
    CustomerCreated(TenantEventDataDetails),
    SubscriptionCreated(TenantEventDataDetails),
    InvoiceCreated(TenantEventDataDetails),
    InvoiceFinalized(TenantEventDataDetails),
}

#[derive(Debug)]
pub struct EventDataDetails {
    pub entity_id: Uuid,
}

#[derive(Debug)]
pub struct TenantEventDataDetails {
    pub tenant_id: Uuid,
    pub entity_id: Uuid,
}
