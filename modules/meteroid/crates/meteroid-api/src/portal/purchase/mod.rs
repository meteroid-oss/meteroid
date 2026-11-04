use std::sync::Arc;

use meteroid_grpc::meteroid::portal::purchase::v1::purchase_service_server::PurchaseServiceServer;
use meteroid_store::Store;

use meteroid_app::services::purchase_links::PurchaseLinks;
use meteroid_app::services::storage::ObjectStoreService;

mod service;

/// Anonymous: callers hold a purchase link token, nothing else.
pub struct PurchaseServiceComponents {
    pub store: Store,
    pub purchases: PurchaseLinks,
    pub object_store: Arc<dyn ObjectStoreService>,
}

pub fn service(
    store: Store,
    purchases: PurchaseLinks,
    object_store: Arc<dyn ObjectStoreService>,
) -> PurchaseServiceServer<PurchaseServiceComponents> {
    PurchaseServiceServer::new(PurchaseServiceComponents {
        store,
        purchases,
        object_store,
    })
}
