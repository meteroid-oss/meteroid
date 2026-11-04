use std::sync::Arc;

use meteroid_grpc::meteroid::api::organizations::v1::organizations_service_server::OrganizationsServiceServer;
use meteroid_middleware::server::auth::SharedSupportGrantReader;
use meteroid_store::Store;

use meteroid_app::features::Entitlements;
use meteroid_kit::Sandboxes;

pub use meteroid_grpc_mapping::organizations::{error, mapping};
mod service;

pub use service::signup_production_tenant;

#[derive(Clone)]
pub struct OrganizationsServiceComponents {
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
    /// Read platform-admin support grants, so a support session can load the
    /// current-org bootstrap read-only instead of 500ing on the membership check.
    support_grants: Option<SharedSupportGrantReader>,
    sandboxes: Arc<dyn Sandboxes>,
}

pub fn service(
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
    support_grants: Option<SharedSupportGrantReader>,
    sandboxes: Arc<dyn Sandboxes>,
) -> OrganizationsServiceServer<OrganizationsServiceComponents> {
    OrganizationsServiceServer::new(OrganizationsServiceComponents {
        store,
        feature_resolver,
        support_grants,
        sandboxes,
    })
}
