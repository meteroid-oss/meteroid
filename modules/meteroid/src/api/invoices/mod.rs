use meteroid_grpc::meteroid::api::invoices::v1::invoices_service_server::InvoicesServiceServer;
use meteroid_store::Store;

mod error;
mod mapping;
mod service;

pub struct InvoiceServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> InvoicesServiceServer<InvoiceServiceComponents> {
    let inner = InvoiceServiceComponents { store };

    InvoicesServiceServer::new(inner)
}
