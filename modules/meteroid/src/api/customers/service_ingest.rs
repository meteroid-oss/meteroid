use crate::api::customers::CustomerIngestServiceComponents;
use crate::api::customers::error::CustomerApiError;
use crate::services::customer_ingest::CsvIngestionOptions;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::customers::v1::customers_ingest_service_server::CustomersIngestService;
use meteroid_grpc::meteroid::api::customers::v1::{
    IngestCsvRequest, IngestCsvResponse, IngestionFailure,
};
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl CustomersIngestService for CustomerIngestServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn ingest_csv(
        &self,
        request: Request<IngestCsvRequest>,
    ) -> Result<Response<IngestCsvResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;

        let req = request.into_inner();

        let file_data = req
            .file
            .ok_or_else(|| Status::invalid_argument("No file provided"))?;

        let options = CsvIngestionOptions {
            delimiter: req.delimiter.chars().next().unwrap_or(','),
            allow_backfilling: false,
            fail_on_error: req.fail_on_error,
        };

        let result = self
            .ingest_service
            .ingest_csv(tenant_id, actor, &file_data.data, options)
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        let failures = result
            .failures
            .into_iter()
            .map(|f| IngestionFailure {
                row_number: f.row_number,
                customer_alias: f.alias,
                reason: f.reason,
            })
            .collect();

        Ok(Response::new(IngestCsvResponse {
            total_rows: result.total_rows,
            successful_rows: result.successful_rows,
            failures,
        }))
    }
}
