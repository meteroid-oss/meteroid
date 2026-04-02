use common_domain::ids::{OrganizationId, TenantId};
use meteroid_grpc::meteroid::admin::deadletter::v1::dead_letter_service_server::DeadLetterService;
use meteroid_grpc::meteroid::admin::deadletter::v1::*;
use meteroid_store::repositories::dead_letter::DeadLetterInterface;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use super::DeadLetterServiceComponents;
use super::error::DeadLetterApiError;
use super::mapping;
use crate::api::shared::platform_admin::require_platform_admin;
use crate::api::utils::PaginationExt;

#[tonic::async_trait]
impl DeadLetterService for DeadLetterServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_dead_letters(
        &self,
        request: Request<ListDeadLettersRequest>,
    ) -> Result<Response<ListDeadLettersResponse>, Status> {
        require_platform_admin(&request, &self.store)?;
        let inner = request.into_inner();

        let status_filter = mapping::from_proto_status(inner.status());
        let tenant_id_filter = inner.tenant_id.map(TenantId::from_proto).transpose()?;
        let organization_id_filter = inner
            .organization_id
            .map(OrganizationId::from_proto)
            .transpose()?;
        let pagination_req = inner.pagination.into_domain();

        let res = self
            .store
            .list_dead_letters(
                inner.queue.as_deref(),
                status_filter,
                tenant_id_filter,
                organization_id_filter,
                pagination_req,
            )
            .await
            .map_err(DeadLetterApiError::from)?;

        Ok(Response::new(ListDeadLettersResponse {
            pagination_meta: inner
                .pagination
                .into_response(res.total_pages, res.total_results),
            entries: res
                .items
                .iter()
                .map(|e| mapping::to_proto_entry(e, None))
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_dead_letter(
        &self,
        request: Request<GetDeadLetterRequest>,
    ) -> Result<Response<GetDeadLetterResponse>, Status> {
        require_platform_admin(&request, &self.store)?;
        let id = Uuid::parse_str(&request.into_inner().id)
            .map_err(|_| DeadLetterApiError::InvalidArgument("Invalid UUID".into()))?;

        let entry = self
            .store
            .get_dead_letter(id)
            .await
            .map_err(DeadLetterApiError::from)?;

        // If requeued, check if the reprocessed message ended up in DLQ again
        let requeued_dlq_id = if let Some(requeued_msg_id) = entry.requeued_pgmq_msg_id {
            self.store
                .find_dead_letter_by_pgmq_msg_id(&entry.queue, requeued_msg_id)
                .await
                .ok()
                .flatten()
                .map(|dlq| dlq.id.to_string())
        } else {
            None
        };

        Ok(Response::new(GetDeadLetterResponse {
            entry: Some(mapping::to_proto_entry(&entry, requeued_dlq_id)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn requeue_dead_letter(
        &self,
        request: Request<RequeueDeadLetterRequest>,
    ) -> Result<Response<RequeueDeadLetterResponse>, Status> {
        let actor = require_platform_admin(&request, &self.store)?;
        let id = Uuid::parse_str(&request.into_inner().id)
            .map_err(|_| DeadLetterApiError::InvalidArgument("Invalid UUID".into()))?;

        let entry = self
            .store
            .requeue_dead_letter(id, actor)
            .await
            .map_err(DeadLetterApiError::from)?;

        Ok(Response::new(RequeueDeadLetterResponse {
            entry: Some(mapping::to_proto_entry(&entry, None)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn discard_dead_letter(
        &self,
        request: Request<DiscardDeadLetterRequest>,
    ) -> Result<Response<DiscardDeadLetterResponse>, Status> {
        let actor = require_platform_admin(&request, &self.store)?;
        let id = Uuid::parse_str(&request.into_inner().id)
            .map_err(|_| DeadLetterApiError::InvalidArgument("Invalid UUID".into()))?;

        let entry = self
            .store
            .discard_dead_letter(id, actor)
            .await
            .map_err(DeadLetterApiError::from)?;

        Ok(Response::new(DiscardDeadLetterResponse {
            entry: Some(mapping::to_proto_entry(&entry, None)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn batch_requeue(
        &self,
        request: Request<BatchRequeueRequest>,
    ) -> Result<Response<BatchRequeueResponse>, Status> {
        let actor = require_platform_admin(&request, &self.store)?;
        let ids: Vec<Uuid> = request
            .into_inner()
            .ids
            .iter()
            .map(|s| Uuid::parse_str(s))
            .collect::<Result<_, _>>()
            .map_err(|_| DeadLetterApiError::InvalidArgument("Invalid UUID in ids".into()))?;

        let count = self
            .store
            .batch_requeue_dead_letters(ids, actor)
            .await
            .map_err(DeadLetterApiError::from)?;

        Ok(Response::new(BatchRequeueResponse {
            requeued_count: count,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn batch_discard(
        &self,
        request: Request<BatchDiscardRequest>,
    ) -> Result<Response<BatchDiscardResponse>, Status> {
        let actor = require_platform_admin(&request, &self.store)?;
        let ids: Vec<Uuid> = request
            .into_inner()
            .ids
            .iter()
            .map(|s| Uuid::parse_str(s))
            .collect::<Result<_, _>>()
            .map_err(|_| DeadLetterApiError::InvalidArgument("Invalid UUID in ids".into()))?;

        let count = self
            .store
            .batch_discard_dead_letters(ids, actor)
            .await
            .map_err(DeadLetterApiError::from)?;

        Ok(Response::new(BatchDiscardResponse {
            discarded_count: count,
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
            queues: stats
                .into_iter()
                .map(mapping::to_proto_queue_health)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn search_organizations(
        &self,
        request: Request<SearchOrganizationsRequest>,
    ) -> Result<Response<SearchOrganizationsResponse>, Status> {
        require_platform_admin(&request, &self.store)?;
        let req = request.into_inner();

        let orgs = self
            .store
            .search_organizations(&req.query, req.limit.clamp(1, 20))
            .await
            .map_err(DeadLetterApiError::from)?;

        Ok(Response::new(SearchOrganizationsResponse {
            organizations: orgs.into_iter().map(mapping::to_proto_org_item).collect(),
        }))
    }
}
