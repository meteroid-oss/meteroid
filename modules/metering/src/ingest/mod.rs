pub mod domain;
mod errors;
mod metrics;
pub mod service;
pub mod sinks;

use crate::ingest::service::EventsService;
use crate::ingest::sinks::Sink;

use common_grpc::middleware::client::LayeredClientService;
use metering_grpc::meteroid::metering::v1::events_service_server::EventsServiceServer;
use meteroid_grpc::meteroid::internal::v1::internal_service_client::InternalServiceClient;
use std::sync::Arc;

pub fn service(
    internal_client: InternalServiceClient<LayeredClientService>,
    sink: Arc<dyn Sink + Send + Sync>,
) -> EventsServiceServer<EventsService> {
    let inner = EventsService::new(internal_client, sink);
    EventsServiceServer::new(inner)
}
