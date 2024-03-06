use std::sync::Arc;

use deadpool_postgres::{Object, Transaction};
use tonic::Status;

use meteroid_grpc::meteroid::api::instance::v1::instance_service_server::InstanceServiceServer;
use meteroid_repository::Pool;

use crate::db::{get_connection, get_transaction};
use crate::eventbus::{Event, EventBus};

mod error;
mod service;

pub struct InstanceServiceComponents {
    pub pool: Pool,
    pub eventbus: Arc<dyn EventBus<Event>>,
}

impl InstanceServiceComponents {
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
) -> InstanceServiceServer<InstanceServiceComponents> {
    let inner = InstanceServiceComponents { pool, eventbus };
    InstanceServiceServer::new(inner)
}
