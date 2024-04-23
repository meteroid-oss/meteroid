use std::sync::Arc;

use meteroid_grpc::meteroid::api::components::v1::price_components_service_server::PriceComponentsServiceServer;

use meteroid_store::Store;

use common_eventbus::{Event, EventBus};

mod error;
pub(crate) mod ext;
pub mod mapping;
mod service;

pub struct PriceComponentServiceComponents {
    pub store: Store,
    pub eventbus: Arc<dyn EventBus<Event>>,
}

pub fn service(
    store: Store,
    eventbus: Arc<dyn EventBus<Event>>,
) -> PriceComponentsServiceServer<PriceComponentServiceComponents> {
    let inner = PriceComponentServiceComponents { store, eventbus };
    PriceComponentsServiceServer::new(inner)
}
