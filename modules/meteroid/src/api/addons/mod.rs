use meteroid_grpc::meteroid::api::addons::v1::add_ons_service_server::AddOnsServiceServer;
use meteroid_store::Store;

mod error;
mod mapping;
mod service;

pub struct AddOnsServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> AddOnsServiceServer<AddOnsServiceComponents> {
    let inner = AddOnsServiceComponents { store };
    AddOnsServiceServer::new(inner)
}
