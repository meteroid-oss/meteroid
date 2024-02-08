use crate::api::services::utils::{uuid_gen, webhook_security};
use crate::api::services::webhooksout::mapping::{endpoint, event_type};
use crate::api::services::webhooksout::WebhooksServiceComponents;
use common_grpc::middleware::server::auth::RequestExt;
use cornucopia_async::Params;
use meteroid_grpc::meteroid::api::webhooks::out::v1::webhooks_service_server::WebhooksService;
use meteroid_grpc::meteroid::api::webhooks::out::v1::{
    CreateWebhookEndpointRequest, CreateWebhookEndpointResponse, ListWebhookEndpointsRequest,
    ListWebhookEndpointsResponse,
};
use meteroid_repository::webhook_out_endpoints::CreateEndpointParams;
use meteroid_repository::WebhookOutEventTypeEnum;
use secrecy::ExposeSecret;
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl WebhooksService for WebhooksServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn create_webhook_endpoint(
        &self,
        request: Request<CreateWebhookEndpointRequest>,
    ) -> Result<Response<CreateWebhookEndpointResponse>, Status> {
        let tenant_id = request.tenant()?.clone();

        let req = request.into_inner();

        let event_types: Vec<WebhookOutEventTypeEnum> = req
            .events_to_listen()
            .map(|e| event_type::to_db(&e))
            .collect();

        url::Url::parse(req.url.as_str())
            .map_err(|e| Status::invalid_argument(format!("Invalid URL: {}", e)))?;

        let secret_raw = webhook_security::gen();
        let secret = crate::crypt::encrypt(&self.crypt_key, secret_raw.as_str())
            .map_err(|x| x.current_context().clone())?;

        let params = CreateEndpointParams {
            id: uuid_gen::v7(),
            tenant_id,
            url: req.url,
            description: req.description,
            secret: secret.expose_secret().to_string(),
            events_to_listen: event_types,
            enabled: true,
        };

        let connection = self.get_connection().await?;

        let created = meteroid_repository::webhook_out_endpoints::create_endpoint()
            .params(&connection, &params)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to create webhook endpoint")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        Ok(Response::new(CreateWebhookEndpointResponse {
            endpoint: Some(endpoint::to_proto(&created, &self.crypt_key)?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_webhook_endpoints(
        &self,
        request: Request<ListWebhookEndpointsRequest>,
    ) -> Result<Response<ListWebhookEndpointsResponse>, Status> {
        let tenant_id = request.tenant()?.clone();

        let connection = self.get_connection().await?;

        let items = meteroid_repository::webhook_out_endpoints::list_endpoints()
            .bind(&connection, &tenant_id)
            .all()
            .await
            .map_err(|e| {
                Status::internal("Unable to list webhook endpoints")
                    .set_source(Arc::new(e))
                    .clone()
            })?
            .iter()
            .map(|e| endpoint::to_proto(e, &self.crypt_key))
            .collect::<Result<_, _>>()?;

        Ok(Response::new(ListWebhookEndpointsResponse {
            endpoints: items,
        }))
    }
}
