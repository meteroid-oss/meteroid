use crate::db::DbService;
use meteroid_grpc::meteroid::api::instance::v1::instance_service_server::InstanceServiceServer;
use meteroid_repository::Pool;

mod service;

pub fn service(pool: Pool) -> InstanceServiceServer<DbService> {
    let inner = DbService::new(pool);
    InstanceServiceServer::new(inner)
}
