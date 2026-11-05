use std::sync::Arc;

use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::api::quotes::v1::quotes_service_server::QuotesServiceServer;
use meteroid_store::{Services, Store};

mod error;
pub mod mapping;
mod service;

pub struct QuoteServiceComponents {
    pub store: Store,
    pub services: Services,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    services: Services,
    feature_resolver: Arc<dyn Entitlements>,
) -> QuotesServiceServer<QuoteServiceComponents> {
    let inner = QuoteServiceComponents {
        store,
        services,
        feature_resolver,
    };

    QuotesServiceServer::new(inner)
}
