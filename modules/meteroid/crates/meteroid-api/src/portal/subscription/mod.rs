use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::portal::subscription::v1::portal_subscription_service_server::PortalSubscriptionServiceServer;
use meteroid_store::{Services, Store};
use secrecy::SecretString;
use std::sync::Arc;

mod error;
mod service;

pub struct PortalSubscriptionServiceComponents {
    pub store: Store,
    pub services: Services,
    pub jwt_secret: SecretString,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    services: Services,
    jwt_secret: SecretString,
    feature_resolver: Arc<dyn Entitlements>,
) -> PortalSubscriptionServiceServer<PortalSubscriptionServiceComponents> {
    let inner = PortalSubscriptionServiceComponents {
        store,
        services,
        jwt_secret,
        feature_resolver,
    };
    PortalSubscriptionServiceServer::new(inner)
}
