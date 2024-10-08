use meteroid_grpc::meteroid::api::billablemetrics::v1::billable_metrics_service_server::BillableMetricsServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

pub struct BillableMetricsComponents {
    pub store: Store,
}

pub fn service(store: Store) -> BillableMetricsServiceServer<BillableMetricsComponents> {
    let inner = BillableMetricsComponents { store };

    BillableMetricsServiceServer::new(inner)
}
