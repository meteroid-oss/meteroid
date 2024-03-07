use crate::db::{get_connection, get_transaction};
use deadpool_postgres::{Object, Pool, Transaction};
use meteroid_grpc::meteroid::api::webhooks::out::v1::webhooks_service_server::WebhooksServiceServer;
use tonic::Status;

mod error;
mod mapping;
mod service;

pub struct WebhooksServiceComponents {
    pub pool: Pool,
    pub crypt_key: secrecy::SecretString,
}

impl WebhooksServiceComponents {
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
    crypt_key: secrecy::SecretString,
) -> WebhooksServiceServer<WebhooksServiceComponents> {
    let inner = WebhooksServiceComponents { pool, crypt_key };
    WebhooksServiceServer::new(inner)
}
