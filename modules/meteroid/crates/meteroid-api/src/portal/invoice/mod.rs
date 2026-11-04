use meteroid_app::features::Entitlements;
use meteroid_app::services::storage::ObjectStoreService;
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
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    services: Services,
    object_store: Arc<dyn ObjectStoreService>,
    jwt_secret: SecretString,
    feature_resolver: Arc<dyn Entitlements>,
) -> PortalInvoiceServiceServer<PortalInvoiceServiceComponents> {
    let inner = PortalInvoiceServiceComponents {
        store,
        services,
        object_store,
        jwt_secret,
        feature_resolver,
    };
    PortalInvoiceServiceServer::new(inner)
}
