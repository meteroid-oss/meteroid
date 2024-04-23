use std::sync::Arc;

use deadpool_postgres::{Object, Transaction};
use tonic::Status;

use meteroid_grpc::meteroid::api::productfamilies::v1::product_families_service_server::ProductFamiliesServiceServer;
use meteroid_repository::Pool;

use crate::db::{get_connection, get_transaction};
use common_eventbus::{Event, EventBus};

mod error;
mod mapping;
mod service;

pub struct ProductFamilyServiceComponents {
    pub pool: Pool,
    pub eventbus: Arc<dyn EventBus<Event>>,
}

impl ProductFamilyServiceComponents {
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
) -> ProductFamiliesServiceServer<ProductFamilyServiceComponents> {
    let inner = ProductFamilyServiceComponents { pool, eventbus };
    ProductFamiliesServiceServer::new(inner)
}
