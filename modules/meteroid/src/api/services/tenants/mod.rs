use crate::db::{get_connection, get_transaction};
use crate::repo::provider_config::ProviderConfigRepo;
use deadpool_postgres::{Object, Transaction};
use meteroid_grpc::meteroid::api::tenants::v1::tenants_service_server::TenantsServiceServer;
use meteroid_repository::Pool;
use std::sync::Arc;
use tonic::Status;

mod mapping;
mod service;

pub struct TenantServiceComponents {
    pub pool: Pool,
    pub provider_config_repo: Arc<dyn ProviderConfigRepo>,
}

impl TenantServiceComponents {
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
    provider_config_repo: Arc<dyn ProviderConfigRepo>,
) -> TenantsServiceServer<TenantServiceComponents> {
    let inner = TenantServiceComponents {
        pool,
        provider_config_repo,
    };

    TenantsServiceServer::new(inner)
}
