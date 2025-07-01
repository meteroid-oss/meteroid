use meteroid_grpc::meteroid::api::webhooks::out::v1::webhooks_service_server::WebhooksServiceServer;
use meteroid_store::{Services, Store};

mod error;
mod mapping;
mod service;

pub struct WebhooksServiceComponents {
    pub store: Store,
    pub services: Services,
}

pub fn service(store: Store, services: Services) -> WebhooksServiceServer<WebhooksServiceComponents> {
    let inner = WebhooksServiceComponents { store, services };
    WebhooksServiceServer::new(inner)
}
