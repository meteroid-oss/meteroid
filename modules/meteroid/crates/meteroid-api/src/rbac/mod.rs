use std::sync::Arc;

use meteroid_grpc::meteroid::api::rbac::v1::rbac_service_server::RbacServiceServer;
use meteroid_store::Store;

use meteroid_app::features::Entitlements;

pub use meteroid_grpc_mapping::rbac::{error, mapping};
mod service;

pub fn service(
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
) -> RbacServiceServer<service::RbacServiceComponents> {
    RbacServiceServer::new(service::RbacServiceComponents {
        store,
        feature_resolver,
    })
}
