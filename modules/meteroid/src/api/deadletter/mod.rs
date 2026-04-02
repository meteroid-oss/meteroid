use meteroid_grpc::meteroid::admin::deadletter::v1::dead_letter_service_server::DeadLetterServiceServer;
use meteroid_store::Store;

mod error;
mod mapping;
mod service;

pub struct DeadLetterServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> DeadLetterServiceServer<DeadLetterServiceComponents> {
    let inner = DeadLetterServiceComponents { store };
    DeadLetterServiceServer::new(inner)
}
