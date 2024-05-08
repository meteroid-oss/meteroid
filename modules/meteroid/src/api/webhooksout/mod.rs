use meteroid_grpc::meteroid::api::webhooks::out::v1::webhooks_service_server::WebhooksServiceServer;
use meteroid_store::Store;

mod error;
mod mapping;
mod service;

pub struct WebhooksServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> WebhooksServiceServer<WebhooksServiceComponents> {
    let inner = WebhooksServiceComponents { store };
    WebhooksServiceServer::new(inner)
}
