use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::api::products::v1::products_service_server::ProductsServiceServer;
use meteroid_store::Store;
use std::sync::Arc;

mod error;
pub(crate) mod mapping;
mod service;

pub struct ProductServiceComponents {
    pub store: Store,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
) -> ProductsServiceServer<ProductServiceComponents> {
    let inner = ProductServiceComponents {
        store,
        feature_resolver,
    };
    ProductsServiceServer::new(inner)
}
