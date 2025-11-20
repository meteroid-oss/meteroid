use crate::services::storage::ObjectStoreService;
use meteroid_grpc::meteroid::portal::checkout::v1::portal_checkout_service_server::PortalCheckoutServiceServer;
use meteroid_store::{Services, Store};
use secrecy::SecretString;
use std::sync::Arc;

mod error;
mod service;

pub struct PortalCheckoutServiceComponents {
    pub store: Store,
    pub services: Services,
    pub object_store: Arc<dyn ObjectStoreService>,
    pub jwt_secret: SecretString,
}

pub fn service(
    store: Store,
    services: Services,
    object_store: Arc<dyn ObjectStoreService>,
    jwt_secret: SecretString,
) -> PortalCheckoutServiceServer<PortalCheckoutServiceComponents> {
    let inner = PortalCheckoutServiceComponents {
        store,
        services,
        object_store,
        jwt_secret,
    };
    PortalCheckoutServiceServer::new(inner)
}
