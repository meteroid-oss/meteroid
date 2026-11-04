use meteroid_grpc::meteroid::api::stats::v1::stats_service_server::StatsServiceServer;
use meteroid_store::Store;
use meteroid_store::repositories::revenue_stats::RevenueStats;
use std::sync::Arc;

mod mapping;
mod service;

pub struct StatsServiceComponents {
    pub store: Store,
    pub revenue_stats: Arc<dyn RevenueStats>,
}

pub fn service(
    store: Store,
    revenue_stats: Arc<dyn RevenueStats>,
) -> StatsServiceServer<StatsServiceComponents> {
    let inner = StatsServiceComponents {
        store,
        revenue_stats,
    };
    StatsServiceServer::new(inner)
}
