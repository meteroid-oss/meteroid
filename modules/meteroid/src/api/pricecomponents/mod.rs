use meteroid_grpc::meteroid::api::components::v1::price_components_service_server::PriceComponentsServiceServer;

use meteroid_store::Store;

mod error;
pub(crate) mod ext;
pub mod mapping;
mod service;

pub struct PriceComponentServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> PriceComponentsServiceServer<PriceComponentServiceComponents> {
    let inner = PriceComponentServiceComponents { store };
    PriceComponentsServiceServer::new(inner)
}
