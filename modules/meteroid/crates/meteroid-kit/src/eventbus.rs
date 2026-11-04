use meteroid_store::Store;

pub use meteroid_app::eventbus::*;

use crate::Modules;
use meteroid_app::config::Config;

/// Call once per process: each subscription receives every event.
pub async fn setup_eventbus_handlers(store: Store, config: Config, modules: &Modules) {
    for module in &modules.api {
        for handler in module.event_handlers(&store, &config) {
            store.eventbus.subscribe(handler).await;
        }
    }
}
