use meteroid_app::features::Entitlements;
use meteroid_app::services::storage::ObjectStoreService;
use meteroid_grpc::meteroid::portal::customer::v1::portal_customer_service_server::PortalCustomerServiceServer;
use meteroid_kit::PortalLicenses;
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
    pub rest_api_external_url: String,
    pub feature_resolver: Arc<dyn Entitlements>,
    /// Absent in an edition without the license issuer: the customer then holds no license.
    pub licenses: Option<Arc<dyn PortalLicenses>>,
}

pub fn service(
    store: Store,
    services: Services,
    object_store: Arc<dyn ObjectStoreService>,
    jwt_secret: SecretString,
    rest_api_external_url: String,
    feature_resolver: Arc<dyn Entitlements>,
    licenses: Option<Arc<dyn PortalLicenses>>,
) -> PortalCustomerServiceServer<PortalCustomerServiceComponents> {
    let inner = PortalCustomerServiceComponents {
        store,
        services,
        object_store,
        jwt_secret,
        rest_api_external_url,
        feature_resolver,
        licenses,
    };
    PortalCustomerServiceServer::new(inner)
}
