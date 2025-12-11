use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::webhooks::out::v1::webhooks_service_server::WebhooksService;
use meteroid_grpc::meteroid::api::webhooks::out::v1::{
    WebhookPortalAccessRequest, WebhookPortalAccessResponse,
};

use crate::api::webhooksout::WebhooksServiceComponents;
use crate::api::webhooksout::error::WebhookApiError;
use crate::svix::SvixOps;

#[tonic::async_trait]
impl WebhooksService for WebhooksServiceComponents {
    async fn get_webhook_portal_access(
        &self,
        request: Request<WebhookPortalAccessRequest>,
    ) -> Result<Response<WebhookPortalAccessResponse>, Status> {
        let tenant_id = request.tenant()?;

        let svix = self
            .svix
            .as_ref()
            .ok_or_else(|| Status::unimplemented("Svix not configured"))?;

        let resp = svix
            .app_portal_access(tenant_id)
            .await
            .map(|x| WebhookPortalAccessResponse {
                access: Some(
                    meteroid_grpc::meteroid::api::webhooks::out::v1::PortalAccess {
                        url: x.url,
                        token: x.token,
                    },
                ),
            })
            .map_err(Into::<WebhookApiError>::into)?;

        Ok(Response::new(resp))
    }
}
