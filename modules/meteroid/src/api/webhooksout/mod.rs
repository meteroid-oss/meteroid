use meteroid_grpc::meteroid::api::webhooks::out::v1::webhooks_service_server::WebhooksServiceServer;
use std::sync::Arc;
use svix::api::Svix;

mod error;
mod service;

pub struct WebhooksServiceComponents {
    pub svix: Option<Arc<Svix>>,
}

pub fn service(svix: Option<Arc<Svix>>) -> WebhooksServiceServer<WebhooksServiceComponents> {
    let inner = WebhooksServiceComponents { svix };
    WebhooksServiceServer::new(inner)
}
