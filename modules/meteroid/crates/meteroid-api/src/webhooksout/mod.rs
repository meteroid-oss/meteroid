use egress_http::EgressClient;
use meteroid_app::config::WebhookOutConfig;
use meteroid_grpc::meteroid::api::webhooks::out::v1::webhooks_service_server::WebhooksServiceServer;
use meteroid_rest_common::webhook_catalog::WebhookCatalog;
use meteroid_store::Store;
use std::sync::Arc;

mod error;
pub mod mapping;
mod service;

pub struct WebhooksServiceComponents {
    pub store: Store,
    pub egress: EgressClient,
    pub webhook_out: WebhookOutConfig,
    /// The events the edition's modules emit, among those only a module emits.
    pub module_events: Vec<&'static str>,
    pub catalog: Arc<dyn WebhookCatalog>,
}

pub fn service(
    store: Store,
    egress: EgressClient,
    webhook_out: WebhookOutConfig,
    module_events: Vec<&'static str>,
    catalog: Arc<dyn WebhookCatalog>,
) -> WebhooksServiceServer<WebhooksServiceComponents> {
    let inner = WebhooksServiceComponents {
        store,
        egress,
        webhook_out,
        module_events,
        catalog,
    };
    WebhooksServiceServer::new(inner)
}
