use crate::db::DbService;
use meteroid_grpc::meteroid::internal::v1::internal_service_server::InternalServiceServer;
use meteroid_repository::Pool;

mod service;

pub fn service(pool: Pool) -> InternalServiceServer<DbService> {
    let inner = DbService::new(pool);
    InternalServiceServer::new(inner)
}
