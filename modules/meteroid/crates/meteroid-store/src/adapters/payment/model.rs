use crate::domain::PaymentMethodTypeEnum;
use chrono::{DateTime, NaiveDateTime, Utc};
use common_domain::ids::PaymentTransactionId;
use secrecy::SecretString;

/// Caller-supplied idempotency key for any mutating provider call.
///
/// **Why it matters.** If the customer's request to Meteroid times out partway
/// through a provider call, our worker may retry the same operation. Without
/// an idempotency key, the provider would create a second customer / charge
/// twice / issue two refunds. Every adapter MUST forward this key to the
/// provider in whatever idiom the provider expects (Stripe: `Idempotency-Key`
/// header; GoCardless: `Idempotency-Key` header; Adyen: `Idempotency-Key`
/// header — they happen to agree).
///
/// The same key submitted twice for the same logical operation must return
/// the original response. The same key submitted for a *different* operation
/// must be rejected by the provider — adapters surface that as
/// `ConnectorError::Configuration` (programming bug, not retryable).
///
/// Construction: callers derive the key from a stable internal identifier
/// (transaction id, refund id, connection id). Random uuids defeat the
/// purpose — every retry would get a new key.
#[derive(Debug, Clone)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identifier returned by the provider for our customer.
/// Opaque string; provider decides the format.
#[derive(Debug, Clone)]
pub struct ExternalCustomerRef {
    pub external_id: String,
    /// The provider's request id for the call that created this customer.
    /// Echoed in support tickets so the provider can correlate.
    pub provider_request_id: Option<String>,
}

/// Snapshot of a payment method as seen by the provider, normalized for storage.
#[derive(Debug, Clone)]
pub struct PaymentMethodSnapshot {
    pub external_payment_method_id: String,
    pub payment_method_type: PaymentMethodTypeEnum,
    pub account_number_hint: Option<String>,
    pub card_brand: Option<String>,
    pub card_last4: Option<String>,
    pub card_exp_month: Option<i32>,
    pub card_exp_year: Option<i32>,
    /// Recovered from the provider resource's metadata when its webhook event
    /// doesn't echo it (GoCardless mandate events carry empty metadata); `None`
    /// for providers whose events already carry our ids (Stripe).
    pub meteroid_connection_id: Option<String>,
    pub meteroid_customer_id: Option<String>,
}

/// How the customer must complete mandate / payment-method setup.
///
/// Providers fall into one of three presentation modes:
/// - `EmbeddedClientSecret` — frontend mounts a provider SDK (Stripe Elements,
///   Adyen Components) using `client_secret` + `publishable_key`.
/// - `HostedRedirect` — frontend redirects the browser to `authorisation_url`;
///   provider hosts the bank-selection / consent UI (GoCardless Billing
///   Request Flow).
/// - `EmbeddedDropIn` — frontend mounts a Drop-in widget initialized with
///   `session_data` (Adyen Drop-in).
#[derive(Debug, Clone)]
pub enum MandateSetupInstruction {
    EmbeddedClientSecret {
        intent_id: String,
        client_secret: String,
        publishable_key: SecretString,
    },
    HostedRedirect {
        intent_id: String,
        authorisation_url: String,
        expires_at: Option<DateTime<Utc>>,
    },
    EmbeddedDropIn {
        intent_id: String,
        session_data: String,
        sdk_version: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct ChargeRequest<'a> {
    pub transaction_id: PaymentTransactionId,
    pub customer_external_id: &'a str,
    pub payment_method_external_id: &'a str,
    pub payment_method_type: PaymentMethodTypeEnum,
    pub amount_minor: i64,
    pub currency: &'a str,
    /// Derived from `transaction_id` — same transaction retried gets the same key.
    pub idempotency_key: IdempotencyKey,
}

/// Outcome of a charge attempt. Normalized across providers so the core code
/// never reasons in terms of Stripe / GoCardless / Adyen statuses.
///
/// Note that an HTTP-level failure (timeout, 5xx) is *not* a `Failed` outcome —
/// it is a `Report<ConnectorError>`. `Failed` means the provider acknowledged
/// the request and refused it (declined). The distinction matters: HTTP errors
/// are retryable with the same idempotency key; `Failed` is terminal.
#[derive(Debug, Clone)]
pub enum ChargeOutcome {
    Succeeded(ChargeReceipt),
    /// The charge was accepted by the provider but settlement is asynchronous
    /// (GoCardless: ~5 business days; Stripe ACH: T+4). Final state arrives via
    /// webhook. The core should mark the transaction `Pending` and wait.
    Pending(ChargeAcknowledged),
    /// Customer interaction is required (3DS, bank app SCA, etc).
    RequiresAction(RequiresActionInstruction),
    /// Cancelled, distinct from a decline — keeps the sync and reconcile paths
    /// consistent and lets dunning treat abandonment differently.
    Cancelled(ChargeCancelled),
    Failed(ChargeFailure),
}

#[derive(Debug, Clone)]
pub struct ChargeReceipt {
    pub external_id: String,
    pub amount_received_minor: i64,
    pub processed_at: NaiveDateTime,
    pub provider_request_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChargeAcknowledged {
    pub external_id: String,
    pub provider_request_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum RequiresActionInstruction {
    HostedUrl {
        external_id: String,
        url: String,
        expires_at: Option<DateTime<Utc>>,
    },
    ClientSecret {
        external_id: String,
        client_secret: String,
        publishable_key: SecretString,
    },
}

#[derive(Debug, Clone)]
pub struct ChargeCancelled {
    pub external_id: Option<String>,
    pub message: String,
    pub provider_request_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChargeFailure {
    pub external_id: Option<String>,
    pub code: Option<String>,
    pub message: String,
    /// Whether retrying the same charge *with the same payment method* is
    /// expected to succeed. False for terminal declines (fraud, expired
    /// card, mandate cancelled); true for transient (issuer timeout,
    /// processor error).
    pub retryable: bool,
    pub decline_kind: DeclineKind,
    pub provider_request_id: Option<String>,
}

/// Coarse categorization of why a charge failed. Used to decide retry policy
/// and customer messaging without leaking provider-specific decline codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclineKind {
    InsufficientFunds,
    DoNotHonor,
    CardExpired,
    AuthenticationRequired,
    MandateInactive,
    Fraud,
    ProcessingError,
    Other,
}

#[derive(Debug, Clone)]
pub struct RefundRequest<'a> {
    pub external_transaction_id: &'a str,
    pub amount_minor: i64,
    pub currency: &'a str,
    pub reason: Option<RefundReason>,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Debug, Clone)]
pub enum RefundReason {
    Duplicate,
    Fraudulent,
    RequestedByCustomer,
    Other(String),
}

#[derive(Debug, Clone)]
pub enum RefundOutcome {
    Succeeded(RefundReceipt),
    Pending(RefundAcknowledged),
    Failed(RefundFailure),
}

#[derive(Debug, Clone)]
pub struct RefundReceipt {
    pub external_refund_id: String,
    pub amount_refunded_minor: i64,
    pub processed_at: NaiveDateTime,
    pub provider_request_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RefundAcknowledged {
    pub external_refund_id: String,
    pub provider_request_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RefundFailure {
    pub code: Option<String>,
    pub message: String,
    pub provider_request_id: Option<String>,
}

/// Result of `WebhookOps::register_webhook`. The returned `secret` is the one
/// the provider will sign future events with — we persist it on the connector.
#[derive(Debug, Clone)]
pub struct RegisteredWebhook {
    pub endpoint_id: String,
    pub secret: SecretString,
}

/// Caller-side request to start mandate setup.
#[derive(Debug, Clone)]
pub struct MandateSetupRequest<'a> {
    pub payment_methods: &'a [PaymentMethodTypeEnum],
    /// Derived from the customer connection id, so a retry after a network
    /// blip returns the same intent rather than creating a duplicate.
    pub idempotency_key: IdempotencyKey,
    /// Where the provider should redirect the browser after the customer
    /// completes (or abandons) the hosted flow. Required for `HostedRedirect`
    /// providers, ignored otherwise.
    pub return_url: Option<String>,
}

/// Caller-side request to create a customer in the provider.
#[derive(Debug, Clone)]
pub struct CreateCustomerRequest {
    /// Derived from our internal customer id. Retries are safe.
    pub idempotency_key: IdempotencyKey,
}

/// Authoritative status of a transaction as known by the provider. Returned by
/// [`super::connector::ReconcileOps::fetch_transaction_status`] when the
/// reconciliation worker checks on a stuck `Pending` transaction.
#[derive(Debug, Clone)]
pub enum RemoteTransactionStatus {
    Succeeded {
        amount_received_minor: i64,
        processed_at: NaiveDateTime,
    },
    Pending,
    Failed {
        code: Option<String>,
        message: String,
        decline_kind: DeclineKind,
    },
    Cancelled,
    /// The provider has no record of this transaction id. Happens when our
    /// outbound call failed before the provider received it — safe to mark
    /// the local transaction as cancelled and retry from scratch.
    Unknown,
}
