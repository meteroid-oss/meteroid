use meteroid::eventbus::{Event, EventBus, EventBusError, EventHandler};
use std::sync::Arc;

pub struct NoopEventBus;

impl NoopEventBus {
    pub fn new() -> Self {
        NoopEventBus
    }
}

#[async_trait::async_trait]
impl EventBus<Event> for NoopEventBus {
    async fn subscribe(&self, _handler: Arc<dyn EventHandler<Event>>) {
        // Noop
    }

    async fn publish(&self, _event: Event) -> Result<(), EventBusError> {
        // Noop
        Ok(())
    }
}
