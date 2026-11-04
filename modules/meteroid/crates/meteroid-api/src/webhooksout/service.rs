use chrono::SecondsFormat;
use tonic::{Request, Response, Status};

use common_domain::ids::{BaseId, EventId, WebhookDeliveryId, WebhookEndpointId};
use common_grpc::middleware::server::auth::RequestExt;
use egress_http::Url;
use meteroid_grpc::meteroid::api::webhooks::out::v1::webhooks_service_server::WebhooksService;
use meteroid_grpc::meteroid::api::webhooks::out::v1::{
    ArchiveWebhookEndpointRequest, ArchiveWebhookEndpointResponse, CreateWebhookEndpointRequest,
    CreateWebhookEndpointResponse, GetWebhookDeliveryRequest, GetWebhookDeliveryResponse,
    GetWebhookEndpointRequest, GetWebhookEndpointResponse, GetWebhookEndpointSecretRequest,
    GetWebhookEndpointSecretResponse, GetWebhookEndpointStatsRequest,
    GetWebhookEndpointStatsResponse, ListWebhookDeliveriesRequest, ListWebhookDeliveriesResponse,
    ListWebhookEndpointStatsRequest, ListWebhookEndpointStatsResponse, ListWebhookEndpointsRequest,
    ListWebhookEndpointsResponse, ListWebhookEventTypesRequest, ListWebhookEventTypesResponse,
    ResendWebhookDeliveryRequest, ResendWebhookDeliveryResponse, ResumeWebhookEndpointRequest,
    ResumeWebhookEndpointResponse, RotateWebhookEndpointSecretRequest,
    RotateWebhookEndpointSecretResponse, SendTestWebhookRequest, SendTestWebhookResponse,
    UpdateWebhookEndpointRequest, UpdateWebhookEndpointResponse, WebhookDailyStats,
    WebhookDeliveryStatusEnum, WebhookEndpointStats, WebhookEventField, WebhookEventType,
};
use meteroid_store::domain::webhooks_out::{
    WebhookEndpointNew, WebhookEndpointPatch, WebhookOutMessageNew,
};
use meteroid_store::repositories::WebhookOutInterface;
use secrecy::ExposeSecret;

use crate::webhooksout::WebhooksServiceComponents;
use crate::webhooksout::error::WebhookApiError;
use crate::webhooksout::mapping::webhooks as mapping;
use meteroid_app::services::webhooks_out::{
    check_endpoint_input, check_headers, check_rate_limit, description_patch, tenant_max_in_flight,
};
use meteroid_grpc_mapping::utils::PaginationExt;

const TEST_EVENT_TYPE: &str = "webhook.test";

/// The window behind the success rate and the deliveries chart.
const STATS_WINDOW: chrono::Duration = chrono::Duration::days(7);

impl WebhooksServiceComponents {
    fn validate_destination(&self, url: &str) -> Result<(), WebhookApiError> {
        let parsed = Url::parse(url)
            .map_err(|e| WebhookApiError::InvalidArgument(format!("invalid url: {e}")))?;

        self.egress
            .validate_url(&parsed)
            .map_err(|e| WebhookApiError::InvalidArgument(format!("url is not allowed: {e}")))
    }

    fn validate_event_types(&self, event_types: &[String]) -> Result<(), WebhookApiError> {
        for event_type in event_types {
            if !self.catalog.is_known(event_type) {
                return Err(WebhookApiError::InvalidArgument(format!(
                    "unknown event type `{event_type}`"
                )));
            }
        }
        Ok(())
    }
}

/// Same envelope shape as a real event, so a consumer's `type` switch and signature
/// verification exercise the exact production path.
fn test_payload(id: EventId, endpoint_id: WebhookEndpointId) -> serde_json::Value {
    serde_json::json!({
        "id": id.as_proto(),
        "type": TEST_EVENT_TYPE,
        "data": { "endpoint_id": endpoint_id.as_proto() },
        "timestamp": chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    })
}

#[tonic::async_trait]
impl WebhooksService for WebhooksServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_webhook_endpoints(
        &self,
        request: Request<ListWebhookEndpointsRequest>,
    ) -> Result<Response<ListWebhookEndpointsResponse>, Status> {
        let tenant_id = request.tenant()?;

        let endpoints = self
            .store
            .list_webhook_endpoints(tenant_id)
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(ListWebhookEndpointsResponse {
            endpoints: endpoints.iter().map(mapping::endpoint_to_server).collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_webhook_endpoint(
        &self,
        request: Request<GetWebhookEndpointRequest>,
    ) -> Result<Response<GetWebhookEndpointResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let id = WebhookEndpointId::from_proto(&req.id)?;

        let endpoint = self
            .store
            .get_webhook_endpoint(tenant_id, id)
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(GetWebhookEndpointResponse {
            endpoint: Some(mapping::endpoint_to_server(&endpoint)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_webhook_endpoint(
        &self,
        request: Request<CreateWebhookEndpointRequest>,
    ) -> Result<Response<CreateWebhookEndpointResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        if req.url.trim().is_empty() {
            return Err(WebhookApiError::MissingArgument("url".to_string()).into());
        }
        check_endpoint_input(
            Some(&req.url),
            req.description.as_deref(),
            Some(&req.event_types),
        )
        .map_err(WebhookApiError::InvalidArgument)?;
        self.validate_destination(&req.url)?;
        self.validate_event_types(&req.event_types)?;

        let headers: Vec<_> = req
            .headers
            .into_iter()
            .map(mapping::header_from_server)
            .collect();
        check_headers(&headers, true).map_err(WebhookApiError::InvalidArgument)?;
        check_rate_limit(req.rate_limit_per_sec).map_err(WebhookApiError::InvalidArgument)?;

        let max_in_flight = tenant_max_in_flight(&self.store, tenant_id)
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        let (endpoint, secret) = self
            .store
            .create_webhook_endpoint(
                tenant_id,
                WebhookEndpointNew {
                    url: req.url,
                    description: req.description.filter(|d| !d.trim().is_empty()),
                    event_types: req.event_types,
                    max_in_flight,
                    headers,
                    rate_limit_per_sec: req.rate_limit_per_sec,
                },
            )
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(CreateWebhookEndpointResponse {
            endpoint: Some(mapping::endpoint_to_server(&endpoint)),
            secret: secret.expose_secret().to_string(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_webhook_endpoint(
        &self,
        request: Request<UpdateWebhookEndpointRequest>,
    ) -> Result<Response<UpdateWebhookEndpointResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let id = WebhookEndpointId::from_proto(&req.id)?;
        let event_types = req.event_types.map(|list| list.event_types);

        check_endpoint_input(
            req.url.as_deref(),
            req.description.as_deref(),
            event_types.as_deref(),
        )
        .map_err(WebhookApiError::InvalidArgument)?;
        if let Some(url) = &req.url {
            self.validate_destination(url)?;
        }
        if let Some(event_types) = &event_types {
            self.validate_event_types(event_types)?;
        }

        let headers: Option<Vec<_>> = req.headers.map(|list| {
            list.headers
                .into_iter()
                .map(mapping::header_from_server)
                .collect()
        });
        if let Some(headers) = &headers {
            check_headers(headers, false).map_err(WebhookApiError::InvalidArgument)?;
        }
        let rate_limit_per_sec = req.rate_limit.map(|limit| limit.per_sec);
        if let Some(rate) = rate_limit_per_sec {
            check_rate_limit(rate).map_err(WebhookApiError::InvalidArgument)?;
        }

        let endpoint = self
            .store
            .update_webhook_endpoint(
                tenant_id,
                WebhookEndpointPatch {
                    id,
                    url: req.url,
                    description: description_patch(req.description),
                    event_types,
                    disabled: req.disabled,
                    headers,
                    rate_limit_per_sec,
                },
            )
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(UpdateWebhookEndpointResponse {
            endpoint: Some(mapping::endpoint_to_server(&endpoint)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn resume_webhook_endpoint(
        &self,
        request: Request<ResumeWebhookEndpointRequest>,
    ) -> Result<Response<ResumeWebhookEndpointResponse>, Status> {
        let tenant_id = request.tenant()?;
        let id = WebhookEndpointId::from_proto(&request.into_inner().id)?;

        let endpoint = self
            .store
            .resume_webhook_endpoint(tenant_id, id)
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(ResumeWebhookEndpointResponse {
            endpoint: Some(mapping::endpoint_to_server(&endpoint)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn archive_webhook_endpoint(
        &self,
        request: Request<ArchiveWebhookEndpointRequest>,
    ) -> Result<Response<ArchiveWebhookEndpointResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let id = WebhookEndpointId::from_proto(&req.id)?;

        self.store
            .archive_webhook_endpoint(tenant_id, id)
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(ArchiveWebhookEndpointResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn get_webhook_endpoint_secret(
        &self,
        request: Request<GetWebhookEndpointSecretRequest>,
    ) -> Result<Response<GetWebhookEndpointSecretResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let id = WebhookEndpointId::from_proto(&req.id)?;

        let secret = self
            .store
            .get_webhook_endpoint_secret(tenant_id, id)
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(GetWebhookEndpointSecretResponse {
            secret: secret.expose_secret().to_string(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn rotate_webhook_endpoint_secret(
        &self,
        request: Request<RotateWebhookEndpointSecretRequest>,
    ) -> Result<Response<RotateWebhookEndpointSecretResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let id = WebhookEndpointId::from_proto(&req.id)?;

        let secret = self
            .store
            .rotate_webhook_endpoint_secret(tenant_id, id)
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(RotateWebhookEndpointSecretResponse {
            secret: secret.expose_secret().to_string(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_webhook_event_types(
        &self,
        _request: Request<ListWebhookEventTypesRequest>,
    ) -> Result<Response<ListWebhookEventTypesResponse>, Status> {
        let event_types = self
            .catalog
            .events()
            .iter()
            .filter(|event| {
                !event.needs_module || self.module_events.contains(&event.name.as_str())
            })
            .map(|event| WebhookEventType {
                name: event.name.clone(),
                group: event.group.clone(),
                beta: event.beta,
                description: Some(event.description.clone()),
                example_json: serde_json::to_string_pretty(&event.example).unwrap_or_default(),
                fields: event
                    .fields
                    .iter()
                    .map(|field| WebhookEventField {
                        path: field.path.clone(),
                        r#type: field.type_name.clone(),
                        description: field.description.clone(),
                        required: field.required,
                    })
                    .collect(),
            })
            .collect();

        Ok(Response::new(ListWebhookEventTypesResponse { event_types }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_webhook_deliveries(
        &self,
        request: Request<ListWebhookDeliveriesRequest>,
    ) -> Result<Response<ListWebhookDeliveriesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let endpoint_id = WebhookEndpointId::from_proto(&req.endpoint_id)?;

        let status = req
            .status
            .map(|status| {
                WebhookDeliveryStatusEnum::try_from(status).map_err(|_| {
                    WebhookApiError::InvalidArgument("invalid delivery status".to_string())
                })
            })
            .transpose()?
            .map(|status| mapping::delivery_status_from_server(&status));

        let res = self
            .store
            .list_webhook_deliveries(tenant_id, endpoint_id, status, req.pagination.into_domain())
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(ListWebhookDeliveriesResponse {
            deliveries: res
                .items
                .iter()
                .map(|item| {
                    mapping::delivery_to_server(
                        &item.delivery,
                        item.event_type.clone(),
                        item.attempts,
                    )
                })
                .collect(),
            pagination_meta: req
                .pagination
                .into_response(res.total_pages, res.total_results),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_webhook_delivery(
        &self,
        request: Request<GetWebhookDeliveryRequest>,
    ) -> Result<Response<GetWebhookDeliveryResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let id = WebhookDeliveryId::from_proto(&req.id)?;

        let details = self
            .store
            .get_webhook_delivery(tenant_id, id)
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(GetWebhookDeliveryResponse {
            delivery: Some(mapping::delivery_to_server(
                &details.delivery,
                details.message.event_type.clone(),
                details.attempts.len() as i64,
            )),
            payload: details.message.payload.to_string(),
            attempts: details
                .attempts
                .iter()
                .map(mapping::attempt_to_server)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn resend_webhook_delivery(
        &self,
        request: Request<ResendWebhookDeliveryRequest>,
    ) -> Result<Response<ResendWebhookDeliveryResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let id = WebhookDeliveryId::from_proto(&req.id)?;

        let details = self
            .store
            .get_webhook_delivery(tenant_id, id)
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        let delivery = self
            .store
            .resend_webhook_delivery(tenant_id, id)
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(ResendWebhookDeliveryResponse {
            delivery: Some(mapping::delivery_to_server(
                &delivery,
                details.message.event_type,
                details.attempts.len() as i64,
            )),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn send_test_webhook(
        &self,
        request: Request<SendTestWebhookRequest>,
    ) -> Result<Response<SendTestWebhookResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let endpoint_id = WebhookEndpointId::from_proto(&req.endpoint_id)?;

        let message_id = EventId::new();
        let (event_type, payload) = match req.event_type {
            Some(event_type) => {
                let timestamp = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
                let payload = self
                    .catalog
                    .example_payload(&event_type, &message_id.as_proto(), &timestamp)
                    .ok_or_else(|| {
                        WebhookApiError::InvalidArgument(format!(
                            "unknown event type `{event_type}`"
                        ))
                    })?;
                (event_type, payload)
            }
            None => (
                TEST_EVENT_TYPE.to_string(),
                test_payload(message_id, endpoint_id),
            ),
        };
        let message = WebhookOutMessageNew {
            id: message_id,
            tenant_id,
            event_type: event_type.clone(),
            payload,
        };

        let delivery = self
            .store
            .create_manual_webhook_delivery(tenant_id, endpoint_id, message)
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(SendTestWebhookResponse {
            delivery: Some(mapping::delivery_to_server(&delivery, event_type, 0)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_webhook_endpoint_stats(
        &self,
        request: Request<GetWebhookEndpointStatsRequest>,
    ) -> Result<Response<GetWebhookEndpointStatsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let endpoint_id = WebhookEndpointId::from_proto(&request.into_inner().endpoint_id)?;

        let days = self
            .store
            .webhook_endpoint_daily_stats(
                tenant_id,
                endpoint_id,
                chrono::Utc::now().naive_utc() - STATS_WINDOW,
            )
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(GetWebhookEndpointStatsResponse {
            days: days
                .into_iter()
                .map(|day| WebhookDailyStats {
                    day: day.day.format("%Y-%m-%d").to_string(),
                    succeeded: day.succeeded,
                    failed: day.failed,
                    retrying: day.retrying,
                    cancelled: day.cancelled,
                })
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_webhook_endpoint_stats(
        &self,
        request: Request<ListWebhookEndpointStatsRequest>,
    ) -> Result<Response<ListWebhookEndpointStatsResponse>, Status> {
        let tenant_id = request.tenant()?;

        let endpoints = self
            .store
            .webhook_endpoints_stats(tenant_id, chrono::Utc::now().naive_utc() - STATS_WINDOW)
            .await
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(ListWebhookEndpointStatsResponse {
            endpoints: endpoints
                .into_iter()
                .map(|stats| WebhookEndpointStats {
                    endpoint_id: stats.endpoint_id.as_proto(),
                    succeeded: stats.succeeded,
                    failed: stats.failed,
                    retrying: stats.retrying,
                    cancelled: stats.cancelled,
                })
                .collect(),
        }))
    }
}
