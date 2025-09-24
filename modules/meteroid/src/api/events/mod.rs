use metering_grpc::meteroid::metering::v1::internal_events_service_client::InternalEventsServiceClient;
use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;
use meteroid_grpc::meteroid::api::events::v1::events_ingestion_service_server::EventsIngestionServiceServer;
use meteroid_store::Store;

mod error;
mod service;

pub struct EventsServiceComponents {
    pub store: Store,
    pub metering_internal_client:
        InternalEventsServiceClient<common_grpc::middleware::client::LayeredClientService>,
    pub metering_query_client:
        UsageQueryServiceClient<common_grpc::middleware::client::LayeredClientService>,
}

pub fn service(
    store: Store,
    metering_internal_client: InternalEventsServiceClient<
        common_grpc::middleware::client::LayeredClientService,
    >,
    metering_query_client: UsageQueryServiceClient<
        common_grpc::middleware::client::LayeredClientService,
    >,
) -> EventsIngestionServiceServer<EventsServiceComponents> {
    let inner = EventsServiceComponents {
        store,
        metering_internal_client,
        metering_query_client,
    };
    EventsIngestionServiceServer::new(inner)
}
