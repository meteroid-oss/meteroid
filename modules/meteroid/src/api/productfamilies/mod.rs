use meteroid_grpc::meteroid::api::productfamilies::v1::product_families_service_server::ProductFamiliesServiceServer;
use meteroid_store::Store;

mod error;
mod mapping;
mod service;

pub struct ProductFamilyServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> ProductFamiliesServiceServer<ProductFamilyServiceComponents> {
    let inner = ProductFamilyServiceComponents { store };
    ProductFamiliesServiceServer::new(inner)
}
