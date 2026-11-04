use crate::domain::enums::ConnectorProviderEnum;
use error_stack::Report;

/// Marker attached to a `complete_mandate_setup` error when the intent exists
/// but its payment method is not populated yet (the redirect can beat the
/// provider's own update). Callers may retry briefly; every other completion
/// error is terminal.
#[derive(Debug, Clone, Copy)]
pub struct HostedSetupPending;

/// Completion error that retrying won't fix (e.g. invalid Mollie mandate); webhook handlers ack
/// it and log. Without it, a `MandateSetup` error stays retryable.
#[derive(Debug, Clone, Copy)]
pub struct HostedSetupFailed;

/// The customer never finished the hosted flow (Mollie `open`). Lets the return handler say
/// "not completed" instead of "still processing".
#[derive(Debug, Clone, Copy)]
pub struct HostedSetupNotCompleted;

/// The provider refused the stored credentials (401/403): revoked, rolled or expired. Attached
/// next to whatever `ConnectorError` the operation maps to, so callers can flag the connector.
#[derive(Debug, Clone, Copy)]
pub struct CredentialsRejected;

/// Whether any frame of `report` carries [`CredentialsRejected`].
pub fn credentials_rejected<C>(report: &error_stack::Report<C>) -> bool {
    report
        .frames()
        .any(|f| f.downcast_ref::<CredentialsRejected>().is_some())
}

/// Customer-facing message; the portal shows it as is.
#[derive(Debug, Clone)]
pub struct CustomerFacingMessage(pub String);

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

    /// The provider refused to create the unconfirmed intent of a wallet top-up (detached
    /// method, unusable card, unsupported provider). Terminal: nothing was charged.
    #[error("Wallet payment preparation rejected: {0}")]
    WalletPreparationRejected(String),

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

/// The provider's own payment for a key reused with a different amount. It is recorded at the
/// provider's amount (never the requested one), so the invoice nets what actually moved.
#[derive(Debug, Clone)]
pub struct AdoptedCharge {
    pub outcome: super::model::ChargeOutcome,
    pub amount_minor: i64,
}

/// The provider already holds a payment under this idempotency key for a different amount.
/// Same currency: the error carries an [`AdoptedCharge`] to record instead of the request.
/// A different currency cannot be reconciled automatically and is left for an operator.
pub(crate) fn adoption_mismatch(
    existing_id: &str,
    existing_amount: i64,
    existing_currency: &str,
    existing_outcome: super::model::ChargeOutcome,
    request: &super::model::ChargeRequest<'_>,
) -> Report<ConnectorError> {
    log::error!(
        "Idempotency key {} already charged {existing_amount} {existing_currency} as {existing_id}, \
         not {} {}",
        request.idempotency_key.as_str(),
        request.amount_minor,
        request.currency
    );
    let report = Report::new(ConnectorError::Charge(format!(
        "existing payment {existing_id} for this idempotency key has a different amount"
    )));
    if existing_currency.eq_ignore_ascii_case(request.currency) {
        report.attach_opaque(AdoptedCharge {
            outcome: existing_outcome,
            amount_minor: existing_amount,
        })
    } else {
        report
    }
}
