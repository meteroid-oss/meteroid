use crate::db::{get_connection, get_transaction};
use crate::eventbus::{Event, EventBus};
use deadpool_postgres::{Object, Transaction};
use meteroid_grpc::meteroid::api::plans::v1::plans_service_server::PlansServiceServer;
use meteroid_repository::Pool;
use std::sync::Arc;
use tonic::Status;

mod error;
mod mapping;
mod service;

pub struct PlanServiceComponents {
    pub pool: Pool,
    pub eventbus: Arc<dyn EventBus<Event>>,
}

impl PlanServiceComponents {
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
) -> PlansServiceServer<PlanServiceComponents> {
    let inner = PlanServiceComponents { pool, eventbus };
    PlansServiceServer::new(inner)
}
