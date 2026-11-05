use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::portal::subscription::v1::portal_subscription_service_server::PortalSubscriptionServiceServer;
use meteroid_store::{Services, Store};
use std::sync::Arc;

mod error;
mod service;

pub struct PortalSubscriptionServiceComponents {
    pub store: Store,
    pub services: Services,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    services: Services,
    feature_resolver: Arc<dyn Entitlements>,
) -> PortalSubscriptionServiceServer<PortalSubscriptionServiceComponents> {
    let inner = PortalSubscriptionServiceComponents {
        store,
        services,
        feature_resolver,
    };
    PortalSubscriptionServiceServer::new(inner)
}
