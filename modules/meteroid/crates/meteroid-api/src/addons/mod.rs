use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::api::addons::v1::add_ons_service_server::AddOnsServiceServer;
use meteroid_store::Store;
use std::sync::Arc;

mod error;
mod mapping;
mod service;

pub struct AddOnsServiceComponents {
    pub store: Store,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
) -> AddOnsServiceServer<AddOnsServiceComponents> {
    let inner = AddOnsServiceComponents {
        store,
        feature_resolver,
    };
    AddOnsServiceServer::new(inner)
}
