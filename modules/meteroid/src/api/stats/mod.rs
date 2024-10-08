use meteroid_grpc::meteroid::api::stats::v1::stats_service_server::StatsServiceServer;
use meteroid_store::Store;

mod mapping;
mod service;

pub struct StatsServiceComponents {
    pub store: Store,
}
pub fn service(store: Store) -> StatsServiceServer<StatsServiceComponents> {
    let inner = StatsServiceComponents { store };
    StatsServiceServer::new(inner)
}
