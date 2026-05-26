//! Programmatic management of Stripe webhook endpoints.
//!
//! Lago-style: when a customer connects their Stripe account, we
//! [`create_webhook_endpoint`] on their behalf using the same API key. Stripe
//! returns a freshly-generated signing `secret` (only on create!) which we
//! persist. The customer never needs to paste a webhook secret manually —
//! reducing the connect-Stripe modal from three fields (api key, publishable
//! key, webhook secret) to two.
//!
//! Requires the `Webhook Endpoints (Write)` permission on the API key, which
//! is the default for full-access keys. Restricted keys can fall back to
//! manual paste (the WebhookOps trait surfaces this via Unsupported when the
//! API call returns 403).

use crate::client::StripeClient;
use crate::error::StripeError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct CreateWebhookEndpointRequest {
    pub url: String,
    /// Stripe event types to subscribe to (e.g. `payment_intent.succeeded`).
    pub enabled_events: Vec<String>,
    /// Surfaces in the Stripe dashboard so the customer can identify the
    /// endpoint we registered for them.
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateWebhookEndpointRequest {
    /// Replaces (not merges) the subscription set on the endpoint. Called when
    /// Meteroid adds new event handlers and existing endpoints need to start
    /// receiving them.
    pub enabled_events: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebhookEndpoint {
    pub id: String,
    pub url: String,
    pub enabled_events: Vec<String>,
    /// The signing secret. Stripe **only returns this on create** — once we
    /// receive it we must persist it; subsequent retrieves leave it `None`.
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
    /// Create a new webhook endpoint. The returned [`WebhookEndpoint::secret`]
    /// is the signing secret for HMAC verification — store it on the
    /// connector immediately, you can't fetch it again.
    async fn create_webhook_endpoint(
        &self,
        params: CreateWebhookEndpointRequest,
        secret_key: &SecretString,
        idempotency_key: String,
    ) -> Result<WebhookEndpoint, StripeError>;

    /// Update the enabled_events on an existing endpoint. Used when we ship
    /// new event handlers and need previously-registered endpoints to start
    /// delivering them.
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
