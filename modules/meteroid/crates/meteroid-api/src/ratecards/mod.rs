use meteroid_grpc::meteroid::api::ratecards::v1::rate_cards_service_server::RateCardsServiceServer;
use meteroid_store::Store;

mod service;

#[derive(Clone)]
pub struct RateCardsServiceComponents {
    store: Store,
}

pub fn service(store: Store) -> RateCardsServiceServer<RateCardsServiceComponents> {
    RateCardsServiceServer::new(RateCardsServiceComponents { store })
}
