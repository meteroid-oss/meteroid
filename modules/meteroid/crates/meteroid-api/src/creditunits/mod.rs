use meteroid_grpc::meteroid::api::creditunits::v1::credit_units_service_server::CreditUnitsServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

#[derive(Clone)]
pub struct CreditUnitsServiceComponents {
    store: Store,
}

pub fn service(store: Store) -> CreditUnitsServiceServer<CreditUnitsServiceComponents> {
    let inner = CreditUnitsServiceComponents { store };
    CreditUnitsServiceServer::new(inner)
}
