use meteroid_app::services::storage::ObjectStoreService;
use meteroid_grpc::meteroid::portal::shared::v1::portal_shared_service_server::PortalSharedServiceServer;
use meteroid_store::{Services, Store};
use rate_limiter::{RateLimitConfig, RateLimiter};
use secrecy::SecretString;
use std::sync::Arc;

mod error;
mod service;

pub struct PortalSharedServiceComponents {
    pub store: Store,
    pub services: Services,
    pub object_store: Arc<dyn ObjectStoreService>,
    pub jwt_secret: SecretString,
    /// Payment-method setups per checkout session (card testing).
    pub checkout_setup_limiter: RateLimiter,
}

const SETUP_INTENTS_PER_CHECKOUT: f64 = 10.0;
const CHECKOUT_SETUP_WINDOW_SECS: u64 = 86_400;

pub fn service(
    store: Store,
    services: Services,
    object_store: Arc<dyn ObjectStoreService>,
    jwt_secret: SecretString,
    rate_limiter: &RateLimiter,
) -> PortalSharedServiceServer<PortalSharedServiceComponents> {
    let checkout_setup_limiter = rate_limiter.with_config(RateLimitConfig {
        capacity: SETUP_INTENTS_PER_CHECKOUT,
        refill_rate: SETUP_INTENTS_PER_CHECKOUT / CHECKOUT_SETUP_WINDOW_SECS as f64,
        window_seconds: CHECKOUT_SETUP_WINDOW_SECS,
        bucket: Some("portal:checkout:setup_intent".to_string()),
    });
    let inner = PortalSharedServiceComponents {
        store,
        services,
        object_store,
        jwt_secret,
        checkout_setup_limiter,
    };
    PortalSharedServiceServer::new(inner)
}
