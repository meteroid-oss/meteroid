pub mod mapping;
pub mod service;

use meteroid_grpc::meteroid::api::taxes::v1::taxes_service_server::TaxesServiceServer;
use meteroid_store::Store;

pub struct TaxesServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> TaxesServiceServer<TaxesServiceComponents> {
    let inner = TaxesServiceComponents { store };
    TaxesServiceServer::new(inner)
}
