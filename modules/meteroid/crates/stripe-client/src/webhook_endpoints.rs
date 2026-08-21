//! Programmatic management of Stripe webhook endpoints: we self-register on the
//! customer's account so they don't paste a secret. Requires the
//! `Webhook Endpoints (Write)` permission; restricted keys get a 403 and must paste manually.

use crate::client::{API_VERSION, StripeClient};
use crate::error::StripeError;
use crate::request::RetryStrategy;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

/// `api_version` is not exposed here: [`WebhookEndpointApi::create_webhook_endpoint`]
/// always pins the endpoint to the crate's `API_VERSION`. Without it, Stripe
/// renders event payloads in the merchant account's default API version, and
/// this crate's webhook deserializers only target the pinned version's shapes.
#[derive(Debug, Clone, Serialize)]
pub struct CreateWebhookEndpointRequest {
    pub url: String,
    pub enabled_events: Vec<String>,
    /// Shown in the Stripe dashboard so the customer can identify this endpoint.
    pub description: Option<String>,
}

#[derive(Serialize)]
struct CreateWebhookEndpointPayload {
    url: String,
    enabled_events: Vec<String>,
    description: Option<String>,
    api_version: &'static str,
}

impl From<CreateWebhookEndpointRequest> for CreateWebhookEndpointPayload {
    fn from(params: CreateWebhookEndpointRequest) -> Self {
        Self {
            url: params.url,
            enabled_events: params.enabled_events,
            description: params.description,
            api_version: API_VERSION,
        }
    }
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
            CreateWebhookEndpointPayload::from(params),
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

#[cfg(test)]
mod tests {
    use super::{CreateWebhookEndpointPayload, CreateWebhookEndpointRequest};
    use crate::client::API_VERSION;
    use serde_qs::Config;

    /// Endpoint creation must pin `api_version`: otherwise Stripe renders
    /// event payloads in the merchant account's default API version, which
    /// can hard-fail this crate's webhook deserializers.
    #[test]
    fn create_webhook_endpoint_pins_api_version() {
        let payload = CreateWebhookEndpointPayload::from(CreateWebhookEndpointRequest {
            url: "https://example.com/webhooks".to_string(),
            enabled_events: vec!["payment_intent.succeeded".to_string()],
            description: Some("Meteroid".to_string()),
        });

        // Same encoding as `StripeClient::post_form`.
        let mut buffer = Vec::new();
        let serializer =
            &mut serde_qs::Serializer::new(&mut buffer, Config::new().use_form_encoding(true));
        serde_path_to_error::serialize(&payload, serializer).unwrap();
        let body = String::from_utf8(buffer).unwrap();

        assert!(
            body.contains(&format!("api_version={API_VERSION}")),
            "body missing pinned api_version: {body}"
        );
    }
}
