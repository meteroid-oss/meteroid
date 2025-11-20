use crate::services::storage::ObjectStoreService;
use meteroid_grpc::meteroid::portal::customer::v1::portal_customer_service_server::PortalCustomerServiceServer;
use meteroid_store::{Services, Store};
use secrecy::SecretString;
use std::sync::Arc;

mod error;
mod service;

pub struct PortalCustomerServiceComponents {
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
) -> PortalCustomerServiceServer<PortalCustomerServiceComponents> {
    let inner = PortalCustomerServiceComponents {
        store,
        services,
        object_store,
        jwt_secret,
    };
    PortalCustomerServiceServer::new(inner)
}