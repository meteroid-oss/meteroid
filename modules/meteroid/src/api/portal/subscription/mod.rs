use meteroid_grpc::meteroid::portal::subscription::v1::portal_subscription_service_server::PortalSubscriptionServiceServer;
use meteroid_store::{Services, Store};

mod error;
mod service;

pub struct PortalSubscriptionServiceComponents {
    pub store: Store,
    pub services: Services,
}

pub fn service(
    store: Store,
    services: Services,
) -> PortalSubscriptionServiceServer<PortalSubscriptionServiceComponents> {
    let inner = PortalSubscriptionServiceComponents { store, services };
    PortalSubscriptionServiceServer::new(inner)
}
