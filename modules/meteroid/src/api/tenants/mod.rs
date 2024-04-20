use crate::repo::provider_config::ProviderConfigRepo;
use meteroid_grpc::meteroid::api::tenants::v1::tenants_service_server::TenantsServiceServer;
use meteroid_store::Store;
use std::sync::Arc;

mod error;
mod mapping;
mod service;

pub struct TenantServiceComponents {
    pub store: Store,
    pub provider_config_repo: Arc<dyn ProviderConfigRepo>,
}

pub fn service(
    store: Store,
    provider_config_repo: Arc<dyn ProviderConfigRepo>,
) -> TenantsServiceServer<TenantServiceComponents> {
    let inner = TenantServiceComponents {
        store,
        provider_config_repo,
    };

    TenantsServiceServer::new(inner)
}
