use crate::db::DbService;
use meteroid_grpc::meteroid::api::customers::v1::customers_service_server::CustomersServiceServer;
use meteroid_repository::Pool;

pub mod mapping;
mod service;

pub fn service(pool: Pool) -> CustomersServiceServer<DbService> {
    let inner = DbService::new(pool);
    CustomersServiceServer::new(inner)
}
