use meteroid_grpc::meteroid::api::components::v1::price_components_service_server::PriceComponentsServiceServer;

use meteroid_app::features::Entitlements;
use meteroid_store::Store;
use std::sync::Arc;

mod error;
pub(crate) mod ext;
pub mod mapping;
mod service;

pub struct PriceComponentServiceComponents {
    pub store: Store,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
) -> PriceComponentsServiceServer<PriceComponentServiceComponents> {
    let inner = PriceComponentServiceComponents {
        store,
        feature_resolver,
    };
    PriceComponentsServiceServer::new(inner)
}
