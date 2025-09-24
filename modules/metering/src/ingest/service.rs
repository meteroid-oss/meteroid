use metering_grpc::meteroid::metering::v1::events_service_server::EventsService as EventsServiceGrpc;
use std::sync::Arc;

use common_grpc::middleware::client::LayeredClientService;
use metering_grpc::meteroid::metering::v1::{IngestRequest, IngestResponse};
use tonic::{Request, Response, Status};

use crate::ingest::common::EventProcessor;
use crate::ingest::sinks::Sink;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::internal::v1::internal_service_client::InternalServiceClient;

#[derive(Clone)]
pub struct EventsService {
    processor: Arc<EventProcessor>,
}

impl EventsService {
    pub fn new(
        internal_client: InternalServiceClient<LayeredClientService>,
        sink: Arc<dyn Sink + Send + Sync>,
    ) -> Self {
        EventsService {
            processor: Arc::new(EventProcessor::new(internal_client, sink)),
        }
    }
}

#[tonic::async_trait]
impl EventsServiceGrpc for EventsService {
    #[tracing::instrument(skip(self, request))]
    async fn ingest(
        &self,
        request: Request<IngestRequest>,
    ) -> Result<Response<IngestResponse>, Status> {
        let tenant_id = request.tenant()?.to_string();
        let req = request.into_inner();

        let result = self
            .processor
            .process_events(req.events, tenant_id, req.allow_backfilling, false)
            .await?;

        Ok(Response::new(IngestResponse {
            failures: result.failures,
        }))
    }
}
