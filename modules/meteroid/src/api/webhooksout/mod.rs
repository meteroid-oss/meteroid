use crate::svix::SvixOps;
use meteroid_grpc::meteroid::api::webhooks::out::v1::webhooks_service_server::WebhooksServiceServer;
use std::sync::Arc;

mod error;
mod service;

pub struct WebhooksServiceComponents {
    pub svix: Option<Arc<dyn SvixOps>>,
}

pub fn service(
    svix: Option<Arc<dyn SvixOps>>,
) -> WebhooksServiceServer<WebhooksServiceComponents> {
    let inner = WebhooksServiceComponents { svix };
    WebhooksServiceServer::new(inner)
}
