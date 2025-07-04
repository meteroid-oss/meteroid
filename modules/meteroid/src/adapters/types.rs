use crate::errors;
use error_stack::Result;
use meteroid_store::Store;
use meteroid_store::domain::connectors::Connector;
use secrecy::SecretString;
use std::fmt::Debug;

pub enum IncomingWebhookEvent {
    EventNotSupported,
}

pub trait AdapterCommon {
    /// Name of the connector (in lowercase).
    fn id(&self) -> &'static str;
}

pub struct ParsedRequest {
    pub method: axum::http::Method,
    pub headers: axum::http::header::HeaderMap,
    pub raw_body: Vec<u8>,
    pub json_body: serde_json::Value,
    pub query_params: Option<String>,
}

#[async_trait::async_trait]
pub trait WebhookAdapter: AdapterCommon + Sync {
    async fn verify_webhook(
        &self,
        request: &ParsedRequest,
        security: &SecretString,
    ) -> Result<bool, errors::AdapterWebhookError>;

    fn get_optimistic_webhook_response(&self) -> axum::response::Response;

    async fn process_webhook_event(
        &self,
        request: &ParsedRequest,
        connector: &Connector,
        store: Store,
    ) -> Result<bool, errors::AdapterWebhookError>;
}

pub trait Adapter: Send + Debug + WebhookAdapter {}
