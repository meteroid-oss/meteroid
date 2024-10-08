use meteroid_grpc::meteroid::api::organizations::v1::organizations_service_server::OrganizationsServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

#[derive(Clone)]
pub struct OrganizationsServiceComponents {
    store: Store,
}

pub fn service(store: Store) -> OrganizationsServiceServer<OrganizationsServiceComponents> {
    let inner = OrganizationsServiceComponents { store };
    OrganizationsServiceServer::new(inner)
}
