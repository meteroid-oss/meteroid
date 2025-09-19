use crate::services::storage::ObjectStoreService;
use meteroid_grpc::meteroid::portal::quotes::v1::portal_quote_service_server::PortalQuoteServiceServer;
use meteroid_store::{Services, Store};
use std::sync::Arc;

mod error;
mod service;

pub struct PortalQuoteServiceComponents {
    pub store: Store,
    pub services: Services,
    pub object_store: Arc<dyn ObjectStoreService>,
}

pub fn service(
    store: Store,
    services: Services,
    object_store: Arc<dyn ObjectStoreService>,
) -> PortalQuoteServiceServer<PortalQuoteServiceComponents> {
    let inner = PortalQuoteServiceComponents {
        store,
        services,
        object_store,
    };
    PortalQuoteServiceServer::new(inner)
}
