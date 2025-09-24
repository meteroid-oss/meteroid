use meteroid_grpc::meteroid::api::events::v1::events_ingestion_service_server::EventsIngestionServiceServer;
use meteroid_store::clients::usage::UsageClient;
use meteroid_store::{Services, Store};
use std::sync::Arc;

mod error;
mod service;

pub struct EventsServiceComponents {
    pub store: Store,
    pub usage_client: Arc<dyn UsageClient>,
}

pub fn service(
    store: Store,
    services: Services,
) -> EventsIngestionServiceServer<EventsServiceComponents> {
    let inner = EventsServiceComponents {
        store,
        usage_client: services.usage_clients(),
    };
    EventsIngestionServiceServer::new(inner)
}
