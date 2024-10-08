use meteroid_grpc::meteroid::api::plans::v1::plans_service_server::PlansServiceServer;
use meteroid_store::Store;

mod error;
mod mapping;
mod service;

pub struct PlanServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> PlansServiceServer<PlanServiceComponents> {
    let inner = PlanServiceComponents { store };
    PlansServiceServer::new(inner)
}
