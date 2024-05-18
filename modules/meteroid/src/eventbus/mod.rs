use std::sync::Arc;

use common_eventbus::{Event, EventBus};
use meteroid_store::Store;

use crate::config::Config;
use crate::eventbus::analytics_handler::AnalyticsHandler;
use crate::eventbus::memory::InMemory;
use crate::eventbus::noop::NoopEventBus;
use crate::eventbus::webhook_handler::WebhookHandler;
use crate::singletons;

pub mod analytics_handler;
pub mod memory;
pub mod noop;
pub mod webhook_handler;

pub async fn create_eventbus_noop() -> Arc<dyn EventBus<Event>> {
    Arc::new(NoopEventBus::new())
}

pub fn create_eventbus_memory() -> Arc<dyn EventBus<Event>> {
    Arc::new(InMemory::new())
}

pub async fn setup_eventbus_handlers(store: Store, config: Config) {
    store
        .clone()
        .eventbus
        .subscribe(Arc::new(WebhookHandler::new(
            store.clone(),
            config.secrets_crypt_key.clone(),
            true,
        )))
        .await;

    if config.analytics.enabled {
        let country = match analytics_handler::get_geoip().await {
            Ok(geoip) => Some(geoip.country),
            Err(err) => {
                log::warn!("Failed to obtain data for analytics: {}", err);
                None
            }
        };

        store
            .clone()
            .eventbus
            .subscribe(Arc::new(AnalyticsHandler::new(
                config.analytics.clone(),
                store.clone(),
                country,
            )))
            .await;
    } else {
        log::info!("Analytics is disabled");
    }
}
