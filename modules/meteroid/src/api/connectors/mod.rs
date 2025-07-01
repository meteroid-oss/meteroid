use meteroid_grpc::meteroid::api::connectors::v1::connectors_service_server::ConnectorsServiceServer;
use meteroid_store::{Services, Store};

mod error;
pub mod mapping;
mod service;

pub struct ConnectorsServiceComponents {
    pub store: Store,
    pub services: Services,
}

pub fn service(
    store: Store,
    services: Services,
) -> ConnectorsServiceServer<ConnectorsServiceComponents> {
    let inner = ConnectorsServiceComponents { store, services };
    ConnectorsServiceServer::new(inner)
}
