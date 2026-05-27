//! Programmatic management of Stripe webhook endpoints: we self-register on the
//! customer's account so they don't paste a secret. Requires the
//! `Webhook Endpoints (Write)` permission; restricted keys get a 403 and must paste manually.

use crate::client::StripeClient;
use crate::error::StripeError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct CreateWebhookEndpointRequest {
    pub url: String,
    pub enabled_events: Vec<String>,
    /// Shown in the Stripe dashboard so the customer can identify this endpoint.
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateWebhookEndpointRequest {
    /// Replaces (not merges) the endpoint's subscription set.
    pub enabled_events: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebhookEndpoint {
    pub id: String,
    pub url: String,
    pub enabled_events: Vec<String>,
    /// Signing secret; Stripe only returns it on create, so persist it then.
    /// Subsequent retrieves leave it `None`.
    #[serde(default)]
    pub secret: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeletedWebhookEndpoint {
    pub id: String,
    pub deleted: bool,
}

#[async_trait::async_trait]
pub trait WebhookEndpointApi {
    /// The returned [`WebhookEndpoint::secret`] is the HMAC signing secret and
    /// can't be fetched again — store it on the connector immediately.
    async fn create_webhook_endpoint(
        &self,
        params: CreateWebhookEndpointRequest,
        secret_key: &SecretString,
        idempotency_key: String,
    ) -> Result<WebhookEndpoint, StripeError>;

    async fn update_webhook_endpoint(
        &self,
        endpoint_id: &str,
        params: UpdateWebhookEndpointRequest,
        secret_key: &SecretString,
        idempotency_key: String,
    ) -> Result<WebhookEndpoint, StripeError>;

    async fn delete_webhook_endpoint(
        &self,
        endpoint_id: &str,
        secret_key: &SecretString,
    ) -> Result<DeletedWebhookEndpoint, StripeError>;
}

#[async_trait::async_trait]
impl WebhookEndpointApi for StripeClient {
    async fn create_webhook_endpoint(
        &self,
        params: CreateWebhookEndpointRequest,
        secret_key: &SecretString,
        idempotency_key: String,
    ) -> Result<WebhookEndpoint, StripeError> {
        self.post_form(
            "/webhook_endpoints",
            params,
            secret_key,
            idempotency_key,
            RetryStrategy::default(),
        )
        .await
    }

    async fn update_webhook_endpoint(
        &self,
        endpoint_id: &str,
        params: UpdateWebhookEndpointRequest,
        secret_key: &SecretString,
        idempotency_key: String,
    ) -> Result<WebhookEndpoint, StripeError> {
        self.post_form(
            &format!("/webhook_endpoints/{endpoint_id}"),
            params,
            secret_key,
            idempotency_key,
            RetryStrategy::default(),
        )
        .await
    }

    async fn delete_webhook_endpoint(
        &self,
        endpoint_id: &str,
        secret_key: &SecretString,
    ) -> Result<DeletedWebhookEndpoint, StripeError> {
        self.delete(
            &format!("/webhook_endpoints/{endpoint_id}"),
            secret_key,
            RetryStrategy::default(),
        )
        .await
    }
}
