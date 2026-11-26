use meteroid_grpc::meteroid::api::resources::v1::resource_types_service_server::ResourceTypesServiceServer;
use meteroid_grpc::meteroid::api::resources::v1::resources_service_server::ResourcesServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

#[derive(Clone)]
pub struct ResourcesServiceComponents {
    store: Store,
}

pub fn types_service(store: Store) -> ResourceTypesServiceServer<ResourcesServiceComponents> {
    ResourceTypesServiceServer::new(ResourcesServiceComponents { store })
}

pub fn service(store: Store) -> ResourcesServiceServer<ResourcesServiceComponents> {
    ResourcesServiceServer::new(ResourcesServiceComponents { store })
}
