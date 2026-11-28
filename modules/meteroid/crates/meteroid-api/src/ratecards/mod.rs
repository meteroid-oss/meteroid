use std::sync::Arc;

use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::api::ratecards::v1::rate_cards_service_server::RateCardsServiceServer;
use meteroid_store::Store;

mod mapping;
mod service;

#[derive(Clone)]
pub struct RateCardsServiceComponents {
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    feature_resolver: Arc<dyn Entitlements>,
) -> RateCardsServiceServer<RateCardsServiceComponents> {
    RateCardsServiceServer::new(RateCardsServiceComponents {
        store,
        feature_resolver,
    })
}
