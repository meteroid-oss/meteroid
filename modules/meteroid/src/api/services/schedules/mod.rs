use meteroid_grpc::meteroid::api::schedules::v1::schedules_service_server::SchedulesServiceServer;
use meteroid_repository::Pool;

use crate::db::DbService;
pub mod mapping;
mod service;

pub fn service(pool: Pool) -> SchedulesServiceServer<DbService> {
    let inner = DbService::new(pool);
    SchedulesServiceServer::new(inner)
}
