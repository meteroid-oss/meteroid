use meteroid_grpc::meteroid::api::prices::v1::prices_service_server::PricesServiceServer;
use meteroid_store::{Services, Store};

mod error;
pub(crate) mod mapping;
mod service;

pub struct PricesServiceComponents {
    pub store: Store,
    pub services: Services,
}

pub fn service(store: Store, services: Services) -> PricesServiceServer<PricesServiceComponents> {
    let inner = PricesServiceComponents { store, services };
    PricesServiceServer::new(inner)
}
