use common_eventbus::EventBus;
use meteroid_grpc::meteroid::api::instance::v1::instance_service_server::InstanceServiceServer;
use meteroid_store::Store;

mod error;
mod service;

pub struct InstanceServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> InstanceServiceServer<InstanceServiceComponents> {
    let inner = InstanceServiceComponents { store };
    InstanceServiceServer::new(inner)
}
