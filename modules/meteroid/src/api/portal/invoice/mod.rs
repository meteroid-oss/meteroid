use crate::services::storage::ObjectStoreService;
use meteroid_grpc::meteroid::portal::invoice::v1::portal_invoice_service_server::PortalInvoiceServiceServer;
use meteroid_store::{Services, Store};
use secrecy::SecretString;
use std::sync::Arc;

mod error;
mod service;

pub struct PortalInvoiceServiceComponents {
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
) -> PortalInvoiceServiceServer<PortalInvoiceServiceComponents> {
    let inner = PortalInvoiceServiceComponents {
        store,
        services,
        object_store,
        jwt_secret,
    };
    PortalInvoiceServiceServer::new(inner)
}
