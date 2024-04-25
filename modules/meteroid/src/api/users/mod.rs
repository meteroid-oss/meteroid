use std::sync::Arc;

use deadpool_postgres::{Object, Pool, Transaction};
use secrecy::SecretString;
use tonic::Status;

use meteroid_grpc::meteroid::api::users::v1::users_service_server::UsersServiceServer;

use crate::db::{get_connection, get_transaction};
use common_eventbus::{Event, EventBus};

mod error;
pub mod mapping;
mod service;

#[derive(Clone)]
pub struct UsersServiceComponents {
    pool: Pool,
    eventbus: Arc<dyn EventBus<Event>>,
    // Or just the specific config fields needed
    jwt_secret: SecretString,
}

impl UsersServiceComponents {
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
    jwt_secret: SecretString,
) -> UsersServiceServer<UsersServiceComponents> {
    let inner = UsersServiceComponents {
        pool,
        eventbus,
        jwt_secret,
    };
    UsersServiceServer::new(inner)
}
