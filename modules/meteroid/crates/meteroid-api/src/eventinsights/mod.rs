use common_domain::ids::TenantId;
use meteroid_grpc::meteroid::api::eventinsights::v1::event_insights_service_server::EventInsightsServiceServer;
use meteroid_store::clients::usage::{DiscoveredProperty, UsageClient};
use meteroid_store::{Services, Store};
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;

mod mapping;
mod service;

/// Property keys by (tenant, event_code, lookback, limit). An event source's keys change slowly,
/// so the editor's autocomplete can fire freely without re-scanning ClickHouse.
pub type PropertySchemaCache = Cache<(TenantId, String, u32, u32), Arc<Vec<DiscoveredProperty>>>;

const PROPERTY_SCHEMA_CACHE_TTL: Duration = Duration::from_secs(600);
const PROPERTY_SCHEMA_CACHE_CAPACITY: u64 = 10_000;

pub struct EventInsightsServiceComponents {
    pub store: Store,
    pub usage_client: Arc<dyn UsageClient>,
    pub property_schema_cache: PropertySchemaCache,
}

pub fn service(
    store: Store,
    services: Services,
) -> EventInsightsServiceServer<EventInsightsServiceComponents> {
    EventInsightsServiceServer::new(EventInsightsServiceComponents {
        store,
        usage_client: services.usage_clients(),
        property_schema_cache: Cache::builder()
            .max_capacity(PROPERTY_SCHEMA_CACHE_CAPACITY)
            .time_to_live(PROPERTY_SCHEMA_CACHE_TTL)
            .build(),
    })
}
