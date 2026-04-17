use crate::services::svix_cache::SvixEndpointCache;
use crate::svix::SvixOps;
use meteroid_grpc::meteroid::api::webhooks::out::v1::webhooks_service_server::WebhooksServiceServer;
use std::sync::Arc;

mod error;
mod service;

pub struct WebhooksServiceComponents {
    pub svix: Option<Arc<dyn SvixOps>>,
    pub endpoint_cache: Arc<dyn SvixEndpointCache>,
}

pub fn service(
    svix: Option<Arc<dyn SvixOps>>,
    endpoint_cache: Arc<dyn SvixEndpointCache>,
) -> WebhooksServiceServer<WebhooksServiceComponents> {
    let inner = WebhooksServiceComponents {
        svix,
        endpoint_cache,
    };
    WebhooksServiceServer::new(inner)
}
