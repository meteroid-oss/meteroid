use crate::services::storage::ObjectStoreService;
use meteroid_grpc::meteroid::portal::checkout::v1::portal_checkout_service_server::PortalCheckoutServiceServer;
use meteroid_store::Store;
use std::sync::Arc;

mod error;
mod service;

pub struct PortalCheckoutServiceComponents {
    pub store: Store,
    pub object_store: Arc<dyn ObjectStoreService>,
}

pub fn service(
    store: Store,
    object_store: Arc<dyn ObjectStoreService>,
) -> PortalCheckoutServiceServer<PortalCheckoutServiceComponents> {
    let inner = PortalCheckoutServiceComponents {
        store,
        object_store,
    };
    PortalCheckoutServiceServer::new(inner)
}
