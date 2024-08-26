use meteroid_grpc::meteroid::api::invoicingentities::v1::invoicing_entities_service_server::InvoicingEntitiesServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

#[derive(Clone)]
pub struct InvoicingEntitiesServiceComponents {
    store: Store,
}

pub fn service(store: Store) -> InvoicingEntitiesServiceServer<InvoicingEntitiesServiceComponents> {
    let inner = InvoicingEntitiesServiceComponents { store };
    InvoicingEntitiesServiceServer::new(inner)
}
