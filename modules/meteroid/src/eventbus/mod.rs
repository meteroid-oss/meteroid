use std::sync::Arc;

use common_eventbus::{Event, EventBus};

use crate::config::Config;
use crate::singletons;

pub mod analytics_handler;
pub mod memory;
pub mod webhook_handler;

static CONFIG: tokio::sync::OnceCell<Arc<dyn EventBus<Event>>> = tokio::sync::OnceCell::const_new();

pub struct EventBusStatic;

impl EventBusStatic {
    pub async fn get() -> &'static Arc<dyn EventBus<Event>> {
        CONFIG
            .get_or_init(|| async {
                let config = Config::get();
                let pool = singletons::get_pool();

                let bus: Arc<dyn EventBus<Event>> = Arc::new(memory::InMemory::new());

                bus.subscribe(Arc::new(webhook_handler::WebhookHandler::new(
                    pool.clone(),
                    config.secrets_crypt_key.clone(),
                    true,
                )))
                .await;

                if config.analytics.enabled {
                    let country = match crate::eventbus::analytics_handler::get_geoip().await {
                        Ok(geoip) => Some(geoip.country),
                        Err(err) => {
                            log::warn!("Failed to obtain data for analytics: {}", err);
                            None
                        }
                    };

                    bus.subscribe(Arc::new(analytics_handler::AnalyticsHandler::new(
                        config.analytics.clone(),
                        pool.clone(),
                        country,
                    )))
                    .await;
                } else {
                    log::info!("Analytics is disabled");
                }

                bus
            })
            .await
    }
}
