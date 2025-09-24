use metering_grpc::meteroid::metering::v1::internal_events_service_server::InternalEventsService as InternalEventsServiceGrpc;
use std::sync::Arc;

use common_grpc::middleware::client::LayeredClientService;
use metering_grpc::meteroid::metering::v1::{InternalIngestRequest, InternalIngestResponse};
use tonic::{Request, Response, Status};

use crate::ingest::common::EventProcessor;
use crate::ingest::sinks::Sink;
use meteroid_grpc::meteroid::internal::v1::internal_service_client::InternalServiceClient;

#[derive(Clone)]
pub struct InternalEventsService {
    processor: Arc<EventProcessor>,
}

impl InternalEventsService {
    pub fn new(
        internal_client: InternalServiceClient<LayeredClientService>,
        sink: Arc<dyn Sink + Send + Sync>,
    ) -> Self {
        InternalEventsService {
            processor: Arc::new(EventProcessor::new(internal_client, sink)),
        }
    }
}

#[tonic::async_trait]
impl InternalEventsServiceGrpc for InternalEventsService {
    #[tracing::instrument(skip(self, request))]
    async fn ingest_internal(
        &self,
        request: Request<InternalIngestRequest>,
    ) -> Result<Response<InternalIngestResponse>, Status> {
        let req = request.into_inner();

        if req.tenant_id.is_empty() {
            return Err(Status::invalid_argument("Tenant ID is required"));
        }

        let result = self
            .processor
            .process_events(
                req.events,
                req.tenant_id,
                req.allow_backfilling,
                req.fail_on_error,
            )
            .await?;

        Ok(Response::new(InternalIngestResponse {
            failures: result.failures,
        }))
    }
}
