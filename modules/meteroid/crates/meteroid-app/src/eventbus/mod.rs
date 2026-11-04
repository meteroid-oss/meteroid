use std::sync::Arc;

use common_eventbus::{Event, EventBus};

use crate::eventbus::memory::InMemory;
use crate::eventbus::noop::NoopEventBus;

pub mod memory;
pub mod noop;

pub fn create_eventbus_noop() -> Arc<dyn EventBus<Event>> {
    Arc::new(NoopEventBus::new())
}

pub fn create_eventbus_memory() -> Arc<dyn EventBus<Event>> {
    Arc::new(InMemory::new())
}
