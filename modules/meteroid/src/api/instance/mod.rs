use meteroid_grpc::meteroid::api::instance::v1::instance_service_server::InstanceServiceServer;
use meteroid_store::Store;

mod error;
mod service;

pub struct InstanceServiceComponents {
    pub store: Store,
    pub svix_enabled: bool,
}

pub fn service(
    store: Store,
    svix_enabled: bool,
) -> InstanceServiceServer<InstanceServiceComponents> {
    let inner = InstanceServiceComponents {
        store,
        svix_enabled,
    };
    InstanceServiceServer::new(inner)
}
