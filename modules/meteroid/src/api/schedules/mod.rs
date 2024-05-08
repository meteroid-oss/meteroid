use meteroid_grpc::meteroid::api::schedules::v1::schedules_service_server::SchedulesServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

pub struct ScheduleServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> SchedulesServiceServer<ScheduleServiceComponents> {
    let inner = ScheduleServiceComponents { store };
    SchedulesServiceServer::new(inner)
}
