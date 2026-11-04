use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::portal::wallets::v1::portal_wallets_service_server::PortalWalletsServiceServer;
use meteroid_store::{Services, Store};
use rate_limiter::RateLimiter;
use std::sync::Arc;

pub(crate) mod mapping;
mod promo_codes;
mod service;

pub(crate) use promo_codes::{CodeTry, PromoCodeLimiter};

pub struct PortalWalletsServiceComponents {
    pub store: Store,
    pub services: Services,
    /// Per customer, so promo codes cannot be guessed by trial.
    pub(crate) promo_codes: PromoCodeLimiter,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    services: Services,
    rate_limiter: &RateLimiter,
    feature_resolver: Arc<dyn Entitlements>,
) -> PortalWalletsServiceServer<PortalWalletsServiceComponents> {
    PortalWalletsServiceServer::new(PortalWalletsServiceComponents {
        store,
        promo_codes: PromoCodeLimiter::new(rate_limiter, services.clone()),
        services,
        feature_resolver,
    })
}
