use meteroid_grpc::meteroid::api::products::v1::products_service_server::ProductsServiceServer;
use meteroid_store::Store;

mod error;
mod mapping;
mod service;

pub struct ProductServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> ProductsServiceServer<ProductServiceComponents> {
    let inner = ProductServiceComponents { store };
    ProductsServiceServer::new(inner)
}
