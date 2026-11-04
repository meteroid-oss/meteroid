use meteroid_grpc::meteroid::api::entitlements::v1::entitlements_service_server::EntitlementsServiceServer;
use meteroid_store::{Services, Store};

mod error;
pub mod mapping;
mod service;

pub struct EntitlementsComponents {
    pub store: Store,
    pub services: Services,
}

pub fn entitlements_service(
    store: Store,
    services: Services,
) -> EntitlementsServiceServer<EntitlementsComponents> {
    EntitlementsServiceServer::new(EntitlementsComponents { store, services })
}
