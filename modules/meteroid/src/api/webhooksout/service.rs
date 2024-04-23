use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::webhooks::out::v1::list_webhook_events_request::SortBy;
use meteroid_grpc::meteroid::api::webhooks::out::v1::webhooks_service_server::WebhooksService;
use meteroid_grpc::meteroid::api::webhooks::out::v1::{
    CreateWebhookEndpointRequest, CreateWebhookEndpointResponse, ListWebhookEndpointsRequest,
    ListWebhookEndpointsResponse, ListWebhookEventsRequest, ListWebhookEventsResponse,
};
use meteroid_store::domain;
use meteroid_store::domain::OrderByRequest;
use meteroid_store::repositories::webhooks::WebhooksInterface;

use crate::api::utils::parse_uuid;
use crate::api::utils::PaginationExt;
use crate::api::webhooksout::error::WebhookApiError;
use crate::api::webhooksout::mapping::{endpoint, event};
use crate::api::webhooksout::WebhooksServiceComponents;

#[tonic::async_trait]
impl WebhooksService for WebhooksServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn create_webhook_endpoint(
        &self,
        request: Request<CreateWebhookEndpointRequest>,
    ) -> Result<Response<CreateWebhookEndpointResponse>, Status> {
        let tenant_id = request.tenant()?.clone();

        let req = request.into_inner();

        let domain = endpoint::new_req_to_domain(tenant_id, req)?;

        let endpoint = self
            .store
            .insert_webhook_out_endpoint(domain)
            .await
            .map(endpoint::to_proto)
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(CreateWebhookEndpointResponse {
            endpoint: Some(endpoint),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_webhook_endpoints(
        &self,
        request: Request<ListWebhookEndpointsRequest>,
    ) -> Result<Response<ListWebhookEndpointsResponse>, Status> {
        let tenant_id = request.tenant()?.clone();

        let items = self
            .store
            .list_webhook_out_endpoints(tenant_id)
            .await
            .map_err(Into::<WebhookApiError>::into)?
            .into_iter()
            .map(endpoint::to_proto)
            .collect();

        Ok(Response::new(ListWebhookEndpointsResponse {
            endpoints: items,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_webhook_events(
        &self,
        request: Request<ListWebhookEventsRequest>,
    ) -> Result<Response<ListWebhookEventsResponse>, Status> {
        let tenant_id = request.tenant()?.clone();

        let req = request.into_inner();

        let endpoint_id = parse_uuid(&req.endpoint_id, "endpoint_id")?;

        let pagination_req = domain::PaginationRequest {
            page: req.pagination.as_ref().map(|p| p.offset).unwrap_or(0),
            per_page: req.pagination.as_ref().map(|p| p.limit),
        };

        let order_by = match req.order_by.try_into() {
            Ok(SortBy::DateAsc) => OrderByRequest::DateAsc,
            Ok(SortBy::DateDesc) => OrderByRequest::DateDesc,
            Ok(SortBy::IdAsc) => OrderByRequest::IdAsc,
            Ok(SortBy::IdDesc) => OrderByRequest::IdDesc,
            Err(_) => OrderByRequest::DateDesc,
        };

        let res = self
            .store
            .list_webhook_out_events(tenant_id, endpoint_id, pagination_req, order_by)
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        let response = ListWebhookEventsResponse {
            pagination_meta: req.pagination.into_response(res.total_results as u32),
            events: res
                .items
                .into_iter()
                .map(|l| event::to_proto(&l))
                .collect::<Vec<_>>(),
        };

        Ok(Response::new(response))
    }
}
