use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::api::creditunits::v1::credit_units_service_server::CreditUnitsServiceServer;
use meteroid_store::Store;
use std::sync::Arc;

mod error;
pub mod mapping;
mod service;

#[derive(Clone)]
pub struct CreditUnitsServiceComponents {
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
) -> CreditUnitsServiceServer<CreditUnitsServiceComponents> {
    let inner = CreditUnitsServiceComponents {
        store,
        feature_resolver,
    };
    CreditUnitsServiceServer::new(inner)
}
