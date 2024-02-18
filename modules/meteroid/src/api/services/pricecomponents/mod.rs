use std::sync::Arc;

use deadpool_postgres::{Object, Transaction};
use tonic::Status;

use meteroid_grpc::meteroid::api::components::v1::price_components_service_server::PriceComponentsServiceServer;
use meteroid_repository::Pool;

use crate::db::{get_connection, get_transaction};
use crate::eventbus::{Event, EventBus};

pub(crate) mod ext;
pub mod mapping;
mod service;

pub struct PriceComponentServiceComponents {
    pub pool: Pool,
    pub eventbus: Arc<dyn EventBus<Event>>,
}

impl PriceComponentServiceComponents {
    pub async fn get_connection(&self) -> Result<Object, Status> {
        get_connection(&self.pool).await
    }
    pub async fn get_transaction<'a>(
        &'a self,
        client: &'a mut Object,
    ) -> Result<Transaction<'a>, Status> {
        get_transaction(client).await
    }
}

pub fn service(
    pool: Pool,
    eventbus: Arc<dyn EventBus<Event>>,
) -> PriceComponentsServiceServer<PriceComponentServiceComponents> {
    let inner = PriceComponentServiceComponents { pool, eventbus };
    PriceComponentsServiceServer::new(inner)
}
