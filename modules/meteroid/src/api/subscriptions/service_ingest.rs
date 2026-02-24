use crate::api::subscriptions::SubscriptionIngestServiceComponents;
use crate::api::subscriptions::error::SubscriptionApiError;
use crate::services::subscription_ingest::SubscriptionIngestionOptions;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_ingest_service_server::SubscriptionsIngestService;
use meteroid_grpc::meteroid::api::subscriptions::v1::{
    IngestCsvRequest, IngestCsvResponse, IngestionFailure,
};
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl SubscriptionsIngestService for SubscriptionIngestServiceComponents {
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

        let options = SubscriptionIngestionOptions {
            delimiter: req.delimiter.chars().next().unwrap_or(','),
            fail_on_error: req.fail_on_error,
        };

        let result = self
            .ingest_service
            .ingest_csv(tenant_id, actor, &file_data.data, options)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let failures = result
            .failures
            .into_iter()
            .map(|f| IngestionFailure {
                row_number: f.row_number,
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
