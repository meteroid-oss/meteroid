use meteroid_grpc::meteroid::api::connectors::v1::connectors_service_server::ConnectorsServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

pub struct ConnectorsServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> ConnectorsServiceServer<ConnectorsServiceComponents> {
    let inner = ConnectorsServiceComponents { store };
    ConnectorsServiceServer::new(inner)
}
