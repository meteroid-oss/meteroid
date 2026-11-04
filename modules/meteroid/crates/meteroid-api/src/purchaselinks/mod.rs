use meteroid_grpc::meteroid::api::purchaselinks::v1::purchase_links_service_server::PurchaseLinksServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

pub struct PurchaseLinksServiceComponents {
    pub store: Store,
    pub public_url: String,
}

pub fn service(
    store: Store,
    public_url: String,
) -> PurchaseLinksServiceServer<PurchaseLinksServiceComponents> {
    let inner = PurchaseLinksServiceComponents {
        store,
        public_url: public_url.trim_end_matches('/').to_string(),
    };
    PurchaseLinksServiceServer::new(inner)
}
