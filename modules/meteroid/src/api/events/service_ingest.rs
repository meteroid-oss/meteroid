use crate::api::events::EventsIngestServiceComponents;
use crate::api::events::error::EventsApiError;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::events::v1::events_ingest_service_server::EventsIngestService;
use meteroid_grpc::meteroid::api::events::v1::{
    IngestCsvRequest, IngestCsvResponse, IngestionFailure,
};
use meteroid_store::clients::usage::CsvIngestionOptions;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl EventsIngestService for EventsIngestServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn ingest_csv(
        &self,
        request: Request<IngestCsvRequest>,
    ) -> Result<Response<IngestCsvResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let file_data = req
            .file
            .ok_or_else(|| Status::invalid_argument("No file provided"))?;

        let options = CsvIngestionOptions {
            delimiter: req.delimiter.chars().next().unwrap_or(','),
            allow_backfilling: req.allow_backfilling,
            fail_on_error: req.fail_on_error,
        };

        let result = self
            .usage_client
            .ingest_events_from_csv(&tenant_id, &file_data.data, options)
            .await
            .map_err(EventsApiError::from)?;

        let failures = result
            .failures
            .into_iter()
            .map(|f| IngestionFailure {
                row_number: f.row_number,
                event_id: f.event_id,
                reason: f.reason,
            })
            .collect();

        Ok(Response::new(IngestCsvResponse {
            total_rows: result.total_rows,
            successful_events: result.successful_events,
            failures,
        }))
    }
}
