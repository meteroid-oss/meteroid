use meteroid_grpc::meteroid::api::pricingpage::v1::pricing_page_service_server::PricingPageServiceServer;
use meteroid_store::Store;

mod mapping;
mod service;

#[derive(Clone)]
pub struct PricingPageServiceComponents {
    store: Store,
}

pub fn service(store: Store) -> PricingPageServiceServer<PricingPageServiceComponents> {
    PricingPageServiceServer::new(PricingPageServiceComponents { store })
}
