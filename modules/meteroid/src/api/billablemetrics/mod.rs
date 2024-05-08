use common_grpc::middleware::client::LayeredClientService;
use metering_grpc::meteroid::metering::v1::meters_service_client::MetersServiceClient;
use meteroid_grpc::meteroid::api::billablemetrics::v1::billable_metrics_service_server::BillableMetricsServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

pub struct BillableMetricsComponents {
    pub store: Store,
    pub meters_service_client: MetersServiceClient<LayeredClientService>,
}

pub fn service(
    store: Store,
    meters_service_client: MetersServiceClient<LayeredClientService>,
) -> BillableMetricsServiceServer<BillableMetricsComponents> {
    let inner = BillableMetricsComponents {
        store,
        meters_service_client,
    };

    BillableMetricsServiceServer::new(inner)
}
