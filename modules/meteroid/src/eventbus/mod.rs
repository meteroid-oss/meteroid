use std::sync::Arc;

use common_eventbus::{Event, EventBus};

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

static EVENTBUS_MEMORY: tokio::sync::OnceCell<Arc<dyn EventBus<Event>>> =
    tokio::sync::OnceCell::const_new();

pub struct EventBusStatic;

pub async fn create_eventbus_noop() -> Arc<dyn EventBus<Event>> {
    Arc::new(NoopEventBus::new())
}

pub async fn create_eventbus_memory(
    pool: deadpool_postgres::Pool,
    config: Config,
) -> Arc<dyn EventBus<Event>> {
    let eventbus = Arc::new(InMemory::new());

    eventbus
        .subscribe(Arc::new(WebhookHandler::new(
            pool.clone(),
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

        eventbus
            .subscribe(Arc::new(AnalyticsHandler::new(
                config.analytics.clone(),
                pool.clone(),
                country,
            )))
            .await;
    } else {
        log::info!("Analytics is disabled");
    }

    eventbus
}

impl EventBusStatic {
    pub async fn get() -> &'static Arc<dyn EventBus<Event>> {
        EVENTBUS_MEMORY
            .get_or_init(|| async {
                let config = Config::get();
                let pool = singletons::get_pool();

                create_eventbus_memory(pool.clone(), config.clone()).await
            })
            .await
    }
}
