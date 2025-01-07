use crate::errors;
use error_stack::Result;
use secrecy::SecretString;
use std::fmt::Debug;

use meteroid_store::domain::{Customer, Invoice};
use meteroid_store::Store;

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
        store: Store,
    ) -> Result<bool, errors::AdapterWebhookError>;
}

#[async_trait::async_trait]
pub trait InvoicingAdapter: AdapterCommon + Sync {
    async fn send_invoice(
        &self,
        invoice: &Invoice,
        customer: &Customer,
        api_key: SecretString,
    ) -> Result<(), errors::InvoicingAdapterError>;
}

pub trait Adapter: Send + Debug + WebhookAdapter {}
