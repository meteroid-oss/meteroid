use meteroid_grpc::meteroid::api::components::v1::price_components_service_server::PriceComponentsServiceServer;
use meteroid_repository::Pool;

use crate::db::DbService;
pub(crate) mod ext;
pub mod mapping;
mod service;

pub fn service(pool: Pool) -> PriceComponentsServiceServer<DbService> {
    let inner = DbService::new(pool);
    PriceComponentsServiceServer::new(inner)
}
