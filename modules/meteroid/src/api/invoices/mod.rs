use crate::db::DbService;
use meteroid_grpc::meteroid::api::invoices::v1::invoices_service_server::InvoicesServiceServer;
use meteroid_repository::Pool;

mod error;
mod mapping;
mod service;

pub fn service(pool: Pool) -> InvoicesServiceServer<DbService> {
    let inner = DbService::new(pool);
    InvoicesServiceServer::new(inner)
}
