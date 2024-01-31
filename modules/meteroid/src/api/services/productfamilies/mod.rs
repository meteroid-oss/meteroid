use crate::db::DbService;
use meteroid_grpc::meteroid::api::productfamilies::v1::product_families_service_server::ProductFamiliesServiceServer;
use meteroid_repository::Pool;

mod mapping;
mod service;

pub fn service(pool: Pool) -> ProductFamiliesServiceServer<DbService> {
    let inner = DbService::new(pool);
    ProductFamiliesServiceServer::new(inner)
}
