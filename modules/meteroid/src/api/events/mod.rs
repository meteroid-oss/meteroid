use meteroid_grpc::meteroid::api::events::v1::events_ingest_service_server::EventsIngestServiceServer;
use meteroid_grpc::meteroid::api::events::v1::events_service_server::EventsServiceServer;
use meteroid_store::clients::usage::UsageClient;
use meteroid_store::{Services, Store};
use std::sync::Arc;

mod error;
mod service;
mod service_ingest;

pub struct EventsServiceComponents {
    pub store: Store,
    pub usage_client: Arc<dyn UsageClient>,
}

pub fn service(store: Store, services: Services) -> EventsServiceServer<EventsServiceComponents> {
    let inner = EventsServiceComponents {
        store,
        usage_client: services.usage_clients(),
    };
    EventsServiceServer::new(inner)
}

pub struct EventsIngestServiceComponents {
    pub usage_client: Arc<dyn UsageClient>,
}

pub fn ingest_service(
    services: Services,
) -> EventsIngestServiceServer<EventsIngestServiceComponents> {
    let inner = EventsIngestServiceComponents {
        usage_client: services.usage_clients(),
    };
    EventsIngestServiceServer::new(inner).max_decoding_message_size(10 * 1024 * 1024) // 10 MB
}
