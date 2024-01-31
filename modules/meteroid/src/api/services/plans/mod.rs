use crate::db::DbService;
use meteroid_grpc::meteroid::api::plans::v1::plans_service_server::PlansServiceServer;
use meteroid_repository::Pool;

mod mapping;
mod service;

pub fn service(pool: Pool) -> PlansServiceServer<DbService> {
    let inner = DbService::new(pool);
    PlansServiceServer::new(inner)
}
