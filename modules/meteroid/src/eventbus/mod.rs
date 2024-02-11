use std::sync::Arc;

pub mod memory;

#[derive(thiserror::Error, Debug)]
pub enum EventBusError {
    #[error("Failed to publish event")]
    PublishFailed,
    #[error("Failed to handle event")]
    EventHandlerFailed(Box<dyn std::error::Error>),
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
