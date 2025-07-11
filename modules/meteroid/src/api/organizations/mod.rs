use meteroid_grpc::meteroid::api::organizations::v1::organizations_service_server::OrganizationsServiceServer;
use meteroid_store::{Services, Store};

mod error;
pub mod mapping;
mod service;

#[derive(Clone)]
pub struct OrganizationsServiceComponents {
    store: Store,
    services: Services,
}

pub fn service(
    store: Store,
    services: Services,
) -> OrganizationsServiceServer<OrganizationsServiceComponents> {
    let inner = OrganizationsServiceComponents { store, services };
    OrganizationsServiceServer::new(inner)
}
