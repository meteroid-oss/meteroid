use crate::services::stats::stats_service::{PgStatsService, StatsService};
use meteroid_grpc::meteroid::api::stats::v1::stats_service_server::StatsServiceServer;
use meteroid_repository::Pool;
use std::sync::Arc;

mod mapping;
mod service;

pub struct StatsServiceComponents {
    pub pool: Pool,
    pub stats_service: Arc<dyn StatsService + Send + Sync>,
}
pub fn service(pool: Pool) -> StatsServiceServer<StatsServiceComponents> {
    let inner = StatsServiceComponents {
        pool: pool.clone(),
        stats_service: Arc::new(PgStatsService::new(pool.clone())),
    };
    StatsServiceServer::new(inner)
}
