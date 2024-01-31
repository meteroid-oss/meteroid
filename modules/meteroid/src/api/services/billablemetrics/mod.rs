use crate::db::{get_connection, get_transaction};

use common_grpc::middleware::client::LayeredClientService;
use deadpool_postgres::{Object, Transaction};
use metering_grpc::meteroid::metering::v1::meters_service_client::MetersServiceClient;
use meteroid_grpc::meteroid::api::billablemetrics::v1::billable_metrics_service_server::BillableMetricsServiceServer;
use meteroid_repository::Pool;
use tonic::Status;

pub mod mapping;
mod service;

pub struct BillableMetricsComponents {
    pub pool: Pool,
    pub meters_service_client: MetersServiceClient<LayeredClientService>,
}

impl BillableMetricsComponents {
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
    meters_service_client: MetersServiceClient<LayeredClientService>,
) -> BillableMetricsServiceServer<BillableMetricsComponents> {
    let inner = BillableMetricsComponents {
        pool,
        meters_service_client,
    };

    BillableMetricsServiceServer::new(inner)
}
