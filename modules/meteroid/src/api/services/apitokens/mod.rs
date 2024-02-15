use std::sync::Arc;

use deadpool_postgres::{Object, Transaction};
use tonic::Status;

use meteroid_grpc::meteroid::api::apitokens::v1::api_tokens_service_server::ApiTokensServiceServer;
use meteroid_repository::Pool;

use crate::db::{get_connection, get_transaction};
use crate::eventbus::{Event, EventBus};

mod mapping;
mod service;

pub struct ApiTokensServiceComponents {
    pub pool: Pool,
    pub eventbus: Arc<dyn EventBus<Event>>,
}

impl ApiTokensServiceComponents {
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
) -> ApiTokensServiceServer<ApiTokensServiceComponents> {
    let inner = ApiTokensServiceComponents { pool, eventbus };
    ApiTokensServiceServer::new(inner)
}
