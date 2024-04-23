use std::sync::Arc;

use common_eventbus::{Event, EventBus};
use deadpool_postgres::{Object, Transaction};
use tonic::Status;

use meteroid_grpc::meteroid::api::customers::v1::customers_service_server::CustomersServiceServer;
use meteroid_repository::Pool;

use crate::db::{get_connection, get_transaction};

pub mod error;
pub mod mapping;
mod service;

pub struct CustomerServiceComponents {
    pub pool: Pool,
    pub eventbus: Arc<dyn EventBus<Event>>,
}

impl CustomerServiceComponents {
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
) -> CustomersServiceServer<CustomerServiceComponents> {
    let inner = CustomerServiceComponents { pool, eventbus };
    CustomersServiceServer::new(inner)
}
