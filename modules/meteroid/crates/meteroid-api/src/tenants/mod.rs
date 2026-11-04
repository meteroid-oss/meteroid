use meteroid_grpc::meteroid::api::tenants::v1::tenants_service_server::TenantsServiceServer;
use meteroid_store::{Services, Store};
use std::sync::Arc;

use meteroid_app::features::Entitlements;
use meteroid_kit::Sandboxes;

pub use meteroid_grpc_mapping::tenants::{error, mapping};
mod service;

pub struct TenantServiceComponents {
    pub store: Store,
    pub services: Services,
    pub feature_resolver: Arc<dyn Entitlements>,
    pub sandboxes: Arc<dyn Sandboxes>,
}

pub fn service(
    store: Store,
    services: Services,
    feature_resolver: Arc<dyn Entitlements>,
    sandboxes: Arc<dyn Sandboxes>,
) -> TenantsServiceServer<TenantServiceComponents> {
    let inner = TenantServiceComponents {
        store,
        services,
        feature_resolver,
        sandboxes,
    };

    TenantsServiceServer::new(inner)
}
