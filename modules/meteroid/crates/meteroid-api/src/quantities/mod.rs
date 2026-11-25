use meteroid_grpc::meteroid::api::quantities::v1::managed_quantities_service_server::ManagedQuantitiesServiceServer;
use meteroid_store::Store;

mod mapping;
mod service;

#[derive(Clone)]
pub struct ManagedQuantitiesServiceComponents {
    store: Store,
}

pub fn service(store: Store) -> ManagedQuantitiesServiceServer<ManagedQuantitiesServiceComponents> {
    ManagedQuantitiesServiceServer::new(ManagedQuantitiesServiceComponents { store })
}
