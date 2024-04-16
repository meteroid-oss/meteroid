use std::sync::Arc;

use deadpool_postgres::{Object, Transaction};
use tonic::Status;

use meteroid_grpc::meteroid::api::components::v1_2::price_components_service_server::PriceComponentsServiceServer;
use meteroid_repository::Pool;
use meteroid_store::Store;

use crate::db::{get_connection, get_transaction};
use crate::eventbus::{Event, EventBus};

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
