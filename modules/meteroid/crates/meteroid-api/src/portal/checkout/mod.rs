use meteroid_app::features::Entitlements;
use meteroid_app::services::storage::ObjectStoreService;
use meteroid_grpc::meteroid::portal::checkout::v1::portal_checkout_service_server::PortalCheckoutServiceServer;
use meteroid_store::{Services, Store};
use rate_limiter::RateLimiter;
use secrecy::SecretString;
use std::sync::Arc;

mod error;
mod service;

pub struct PortalCheckoutServiceComponents {
    pub store: Store,
    pub services: Services,
    pub object_store: Arc<dyn ObjectStoreService>,
    pub jwt_secret: SecretString,
    /// Per customer, so promo codes cannot be guessed by trial.
    pub(crate) promo_codes: crate::portal::wallets::PromoCodeLimiter,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    services: Services,
    object_store: Arc<dyn ObjectStoreService>,
    jwt_secret: SecretString,
    rate_limiter: &RateLimiter,
    feature_resolver: Arc<dyn Entitlements>,
) -> PortalCheckoutServiceServer<PortalCheckoutServiceComponents> {
    let inner = PortalCheckoutServiceComponents {
        store,
        promo_codes: crate::portal::wallets::PromoCodeLimiter::new(rate_limiter, services.clone()),
        services,
        object_store,
        jwt_secret,
        feature_resolver,
    };
    PortalCheckoutServiceServer::new(inner)
}
