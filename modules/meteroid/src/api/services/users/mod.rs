use crate::db::DbService;
use meteroid_grpc::meteroid::api::users::v1::users_service_server::UsersServiceServer;
use secrecy::SecretString;

mod mapping;
mod service;

use deadpool_postgres::{Object, Pool, Transaction};

use tonic::Status;

pub fn service(pool: Pool, jwt_secret: SecretString) -> UsersServiceServer<UsersDbService> {
    let inner = UsersDbService::new(DbService::new(pool), jwt_secret);
    UsersServiceServer::new(inner)
}

#[derive(Clone)]
pub struct UsersDbService {
    db_service: DbService,
    jwt_secret: SecretString, // Or just the specific config fields needed
}

impl UsersDbService {
    pub fn new(db_service: DbService, jwt_secret: SecretString) -> Self {
        Self {
            db_service,
            jwt_secret,
        }
    }

    // Delegate
    pub async fn get_connection(&self) -> Result<Object, Status> {
        self.db_service.get_connection().await
    }
    pub async fn get_transaction<'a>(
        &'a self,
        client: &'a mut Object,
    ) -> Result<Transaction<'a>, Status> {
        self.db_service.get_transaction(client).await
    }
}
