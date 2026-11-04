use meteroid_grpc::meteroid::api::plans::v1::plans_service_server::PlansServiceServer;
use meteroid_store::Store;
use std::sync::Arc;

use meteroid_app::features::Entitlements;

mod error;
pub(crate) mod mapping;
mod service;

pub struct PlanServiceComponents {
    pub store: Store,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
) -> PlansServiceServer<PlanServiceComponents> {
    let inner = PlanServiceComponents {
        store,
        feature_resolver,
    };
    PlansServiceServer::new(inner)
}
