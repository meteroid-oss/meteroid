use crate::services::storage::ObjectStoreService;
use meteroid_grpc::meteroid::portal::shared::v1::portal_shared_service_server::PortalSharedServiceServer;
use meteroid_store::{Services, Store};
use secrecy::SecretString;
use std::sync::Arc;

mod error;
mod service;

pub struct PortalSharedServiceComponents {
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
) -> PortalSharedServiceServer<PortalSharedServiceComponents> {
    let inner = PortalSharedServiceComponents {
        store,
        services,
        object_store,
        jwt_secret,
    };
    PortalSharedServiceServer::new(inner)
}
