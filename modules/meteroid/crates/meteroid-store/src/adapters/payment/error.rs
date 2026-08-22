use crate::domain::enums::ConnectorProviderEnum;

/// Marker attached to a `complete_mandate_setup` error when the intent exists
/// but its payment method is not populated yet (the redirect can beat the
/// provider's own update). Callers may retry briefly; every other completion
/// error is terminal.
#[derive(Debug, Clone, Copy)]
pub struct HostedSetupPending;

#[derive(Debug, thiserror::Error)]
pub enum ConnectorError {
    #[error("Connector configuration error: {0}")]
    Configuration(String),

    #[error("Provider {provider:?} does not support {capability}")]
    Unsupported {
        provider: ConnectorProviderEnum,
        capability: &'static str,
    },

    #[error("Customer operation failed: {0}")]
    CustomerOp(String),

    #[error("Mandate setup failed: {0}")]
    MandateSetup(String),

    #[error("Charge failed: {0}")]
    Charge(String),

    #[error("Refund failed: {0}")]
    Refund(String),

    #[error("Webhook registration failed: {0}")]
    WebhookRegistration(String),

    #[error("Webhook signature verification failed")]
    SignatureVerification,

    #[error("Webhook signature header missing")]
    SignatureMissing,

    #[error("Webhook payload decode failed: {0}")]
    PayloadDecode(String),

    #[error("Webhook event missing metadata field: {0}")]
    MissingMetadata(String),

    #[error("Webhook event has invalid metadata: {0}")]
    InvalidMetadata(String),

    #[error("Database error")]
    Database,

    #[error("Underlying transport error: {0}")]
    Transport(String),
}
