use crate::db::{get_connection, get_transaction};
use deadpool_postgres::{Object, Transaction};
use meteroid_grpc::meteroid::api::plans::v1::plans_service_server::PlansServiceServer;
use meteroid_repository::Pool;
use meteroid_store::Store;
use tonic::Status;

mod error;
mod mapping;
mod service;

pub struct PlanServiceComponents {
    #[deprecated]
    pub pool: Pool,
    pub store: Store,
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

pub fn service(pool: Pool, store: Store) -> PlansServiceServer<PlanServiceComponents> {
    let inner = PlanServiceComponents { pool, store };
    PlansServiceServer::new(inner)
}
