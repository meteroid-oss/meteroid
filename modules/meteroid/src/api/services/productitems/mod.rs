use crate::db::DbService;
use meteroid_grpc::meteroid::api::products::v1::products_service_server::ProductsServiceServer;
use meteroid_repository::Pool;

mod mapping;
mod service;

pub fn service(pool: Pool) -> ProductsServiceServer<DbService> {
    let inner = DbService::new(pool);
    ProductsServiceServer::new(inner)
}
