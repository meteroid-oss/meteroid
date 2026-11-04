use meteroid_app::services::storage::ObjectStoreService;
use meteroid_grpc::meteroid::api::invoicingentities::v1::invoicing_entities_service_server::InvoicingEntitiesServiceServer;
use meteroid_store::Store;
use std::sync::Arc;

pub use meteroid_grpc_mapping::invoicingentities::{error, mapping};
mod service;

#[derive(Clone)]
pub struct InvoicingEntitiesServiceComponents {
    store: Store,
    object_store: Arc<dyn ObjectStoreService>,
}

pub fn service(
    store: Store,
    object_store: Arc<dyn ObjectStoreService>,
) -> InvoicingEntitiesServiceServer<InvoicingEntitiesServiceComponents> {
    InvoicingEntitiesServiceServer::new(InvoicingEntitiesServiceComponents {
        store,
        object_store,
    })
}
