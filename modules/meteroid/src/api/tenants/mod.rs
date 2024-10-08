use meteroid_grpc::meteroid::api::tenants::v1::tenants_service_server::TenantsServiceServer;
use meteroid_store::Store;

mod error;
pub(crate) mod mapping;
mod service;

pub struct TenantServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> TenantsServiceServer<TenantServiceComponents> {
    let inner = TenantServiceComponents { store };

    TenantsServiceServer::new(inner)
}
