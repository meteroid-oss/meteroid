use meteroid_grpc::meteroid::admin::deadletter::v1::dead_letter_service_server::DeadLetterService;
use meteroid_grpc::meteroid::admin::deadletter::v1::*;
use meteroid_store::repositories::dead_letter::DeadLetterInterface;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use super::DeadLetterServiceComponents;
use super::error::DeadLetterApiError;
use super::mapping;
use crate::api::shared::platform_admin::require_platform_admin;

#[tonic::async_trait]
impl DeadLetterService for DeadLetterServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_dead_letters(
        &self,
        request: Request<ListDeadLettersRequest>,
    ) -> Result<Response<ListDeadLettersResponse>, Status> {
        require_platform_admin(&request, &self.store)?;
        let req = request.into_inner();

        let status_filter =
            mapping::from_proto_status(req.status());

        let (entries, total_count) = self
            .store
            .list_dead_letters(
                req.queue.as_deref(),
                status_filter,
                req.limit as i64,
                req.offset as i64,
            )
            .await
            .map_err(DeadLetterApiError::from)?;

        Ok(Response::new(ListDeadLettersResponse {
            entries: entries.into_iter().map(mapping::to_proto_entry).collect(),
            total_count,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_dead_letter(
        &self,
        request: Request<GetDeadLetterRequest>,
    ) -> Result<Response<GetDeadLetterResponse>, Status> {
        require_platform_admin(&request, &self.store)?;
        let req = request.into_inner();

        let id = Uuid::parse_str(&req.id)
            .map_err(|_| DeadLetterApiError::InvalidArgument("Invalid UUID".to_string()))?;

        let entry = self
            .store
            .get_dead_letter(id)
            .await
            .map_err(DeadLetterApiError::from)?;

        Ok(Response::new(GetDeadLetterResponse {
            entry: Some(mapping::to_proto_entry(entry)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn requeue_dead_letter(
        &self,
        request: Request<RequeueDeadLetterRequest>,
    ) -> Result<Response<RequeueDeadLetterResponse>, Status> {
        let actor = require_platform_admin(&request, &self.store)?;
        let req = request.into_inner();

        let id = Uuid::parse_str(&req.id)
            .map_err(|_| DeadLetterApiError::InvalidArgument("Invalid UUID".to_string()))?;

        let entry = self
            .store
            .requeue_dead_letter(id, actor)
            .await
            .map_err(DeadLetterApiError::from)?;

        Ok(Response::new(RequeueDeadLetterResponse {
            entry: Some(mapping::to_proto_entry(entry)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn discard_dead_letter(
        &self,
        request: Request<DiscardDeadLetterRequest>,
    ) -> Result<Response<DiscardDeadLetterResponse>, Status> {
        let actor = require_platform_admin(&request, &self.store)?;
        let req = request.into_inner();

        let id = Uuid::parse_str(&req.id)
            .map_err(|_| DeadLetterApiError::InvalidArgument("Invalid UUID".to_string()))?;

        let entry = self
            .store
            .discard_dead_letter(id, actor)
            .await
            .map_err(DeadLetterApiError::from)?;

        Ok(Response::new(DiscardDeadLetterResponse {
            entry: Some(mapping::to_proto_entry(entry)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_queue_health(
        &self,
        request: Request<GetQueueHealthRequest>,
    ) -> Result<Response<GetQueueHealthResponse>, Status> {
        require_platform_admin(&request, &self.store)?;

        let stats = self
            .store
            .dead_letter_queue_stats()
            .await
            .map_err(DeadLetterApiError::from)?;

        Ok(Response::new(GetQueueHealthResponse {
            queues: stats.into_iter().map(mapping::to_proto_queue_health).collect(),
        }))
    }
}
