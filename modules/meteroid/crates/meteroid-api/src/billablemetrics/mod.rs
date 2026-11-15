use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::api::billablemetrics::v1::billable_metrics_service_server::BillableMetricsServiceServer;
use meteroid_store::Store;
use meteroid_store::services::clients::usage::UsageClient;
use std::sync::Arc;

mod error;
pub mod mapping;
mod service;

pub struct BillableMetricsComponents {
    pub store: Store,
    pub usage_client: Arc<dyn UsageClient>,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    usage_client: Arc<dyn UsageClient>,
    feature_resolver: Arc<dyn Entitlements>,
) -> BillableMetricsServiceServer<BillableMetricsComponents> {
    let inner = BillableMetricsComponents {
        store,
        usage_client,
        feature_resolver,
    };

    BillableMetricsServiceServer::new(inner)
}
