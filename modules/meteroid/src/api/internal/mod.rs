use meteroid_grpc::meteroid::internal::v1::internal_service_server::InternalServiceServer;
use meteroid_store::Store;

mod error;
mod service;

pub struct InternalServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> InternalServiceServer<InternalServiceComponents> {
    let inner = InternalServiceComponents { store };
    InternalServiceServer::new(inner)
}
