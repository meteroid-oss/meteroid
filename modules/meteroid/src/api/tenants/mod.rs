use meteroid_grpc::meteroid::api::tenants::v1::tenants_service_server::TenantsServiceServer;
use meteroid_store::{Services, Store};

mod error;
pub(crate) mod mapping;
mod service;

pub struct TenantServiceComponents {
    pub store: Store,
    pub services: Services,
}

pub fn service(store: Store, services: Services) -> TenantsServiceServer<TenantServiceComponents> {
    let inner = TenantServiceComponents { store, services };

    TenantsServiceServer::new(inner)
}
