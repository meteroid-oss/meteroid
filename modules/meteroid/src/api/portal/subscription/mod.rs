use meteroid_grpc::meteroid::portal::subscription::v1::portal_subscription_service_server::PortalSubscriptionServiceServer;
use meteroid_store::{Services, Store};
use secrecy::SecretString;

mod error;
mod service;

pub struct PortalSubscriptionServiceComponents {
    pub store: Store,
    pub services: Services,
    pub jwt_secret: SecretString,
}

pub fn service(
    store: Store,
    services: Services,
    jwt_secret: SecretString,
) -> PortalSubscriptionServiceServer<PortalSubscriptionServiceComponents> {
    let inner = PortalSubscriptionServiceComponents {
        store,
        services,
        jwt_secret,
    };
    PortalSubscriptionServiceServer::new(inner)
}
