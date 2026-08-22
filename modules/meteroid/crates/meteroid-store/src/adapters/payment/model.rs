use crate::domain::PaymentMethodTypeEnum;
use chrono::{DateTime, NaiveDateTime, Utc};
use common_domain::ids::PaymentTransactionId;
use secrecy::SecretString;

/// Caller-supplied idempotency key for any mutating provider call; adapters MUST
/// forward it to the provider. Derive it from a stable internal id (random uuids
/// defeat the purpose — every retry would get a new key). Reuse for a *different*
/// operation is rejected by the provider, surfaced as `Configuration`.
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

#[derive(Debug, Clone)]
pub struct ExternalCustomerRef {
    pub external_id: String,
    /// Provider request id, echoed in support tickets for correlation.
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
    /// Recovered from provider metadata when the webhook event doesn't echo it
    /// (GoCardless); `None` when events already carry our ids (Stripe).
    pub meteroid_connection_id: Option<String>,
    pub meteroid_customer_id: Option<String>,
    /// Invoice this mandate was set up to pay (base62 `InvoiceId`), recovered
    /// from the mandate's metadata. Drives the post-mandate charge for
    /// hosted-redirect providers. `None` for plain add-a-payment-method setups.
    pub meteroid_invoice_id: Option<String>,
    /// Checkout session this mandate was set up to complete (base62
    /// `CheckoutSessionId`), recovered from the Billing Request metadata. Set on
    /// a hosted checkout where the Billing Request also carried a `payment_request`
    /// (combined mandate + first payment). Drives in-flight subscription
    /// activation on `billing_requests.fulfilled`. `None` otherwise.
    pub meteroid_checkout_session_id: Option<String>,
    /// base62 id of the Pending transaction this setup was minted for
    /// (`meteroid.transaction_id`). Completion records the captured payment
    /// onto THIS transaction — never "the latest", which can belong to a newer
    /// attempt. `None` for non-checkout setups and legacy unstamped intents.
    pub meteroid_transaction_id: Option<String>,
    /// The provider payment created from the Billing Request's `payment_request`
    /// (GoCardless `links.payment_request_payment`), present only on a combined
    /// mandate+payment hosted checkout. Recorded as the checkout transaction's
    /// `provider_transaction_id` so the later `payments.*` webhooks settle it.
    pub payment_request_payment: Option<String>,
}

/// How the customer must complete setup: mount a provider SDK with
/// `client_secret` (Stripe), redirect to `authorisation_url` (GoCardless), or
/// mount a drop-in with `session_data` (Adyen).
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
    /// Derived from `transaction_id` so retries reuse the same key.
    pub idempotency_key: IdempotencyKey,
    /// Whether the customer is present in a browser. On-session lets the
    /// provider return a completable `requires_action` (3DS) the portal can
    /// finish; off-session (recurring) charges can only be flagged for later.
    pub on_session: bool,
}

/// Normalized charge outcome across providers. A `Failed` here is a provider
/// decline (terminal); an HTTP-level failure is a `Report<ConnectorError>`
/// instead (retryable with the same idempotency key).
#[derive(Debug, Clone)]
pub enum ChargeOutcome {
    Succeeded(ChargeReceipt),
    /// Accepted but settlement is asynchronous; final state arrives via webhook,
    /// so mark the transaction `Pending` and wait.
    Pending(ChargeAcknowledged),
    /// Customer interaction required (3DS, bank app SCA).
    RequiresAction(RequiresActionInstruction),
    /// Distinct from a decline so dunning can treat abandonment differently.
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
    /// Whether retrying with the same payment method may succeed: false for
    /// terminal declines (fraud, expired card), true for transient ones.
    pub retryable: bool,
    pub decline_kind: DeclineKind,
    pub provider_request_id: Option<String>,
}

/// Coarse failure category for retry policy and messaging, without leaking
/// provider-specific decline codes.
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

/// A webhook-observed refund resolved at the provider (the event itself carried
/// no amounts — GoCardless refund events only link the refund id).
#[derive(Debug, Clone)]
pub struct RefundSnapshot {
    /// Provider id of the parent payment/charge.
    pub external_transaction_id: String,
    /// The parent payment's cumulative refunded total, in minor units.
    pub cumulative_refunded_minor: i64,
    pub currency: String,
}

/// `secret` is the signing secret for future events; we persist it on the connector.
#[derive(Debug, Clone)]
pub struct RegisteredWebhook {
    pub endpoint_id: String,
    pub secret: SecretString,
}

#[derive(Debug, Clone)]
pub struct MandateSetupRequest<'a> {
    pub payment_methods: &'a [PaymentMethodTypeEnum],
    /// Derived from the connection id so a retry returns the same intent.
    pub idempotency_key: IdempotencyKey,
    /// Post-flow redirect target; required for `HostedRedirect`, ignored otherwise.
    pub return_url: Option<String>,
    /// Set when the mandate is being set up to pay a specific invoice. Stored in
    /// the provider's Billing Request metadata (base62 `InvoiceId`) so that — for
    /// hosted-redirect providers where the charge can only happen after the
    /// mandate exists — the `billing_requests.fulfilled` webhook can recover it
    /// and charge the invoice. `None` for a plain "add a payment method".
    pub invoice_id: Option<String>,
    /// Set for a hosted CHECKOUT: the provider is asked to collect the first
    /// payment together with the mandate in a single hosted flow (GoCardless
    /// Billing Request with both a `mandate_request` and a `payment_request`).
    /// Mutually exclusive with `invoice_id` (checkout has no invoice yet).
    pub checkout: Option<HostedCheckoutContext>,
    /// Hosted INVOICE payment on an in-flow-capturing (PollingRequired)
    /// provider: the hosted page captures the real `amount_due` with the card
    /// save — the single charge; completion records it, never re-charges.
    /// Mutually exclusive with `checkout`; webhook-backed providers never
    /// receive it.
    pub invoice_payment: Option<HostedInvoicePaymentContext>,
    /// The customer's billing currency (ISO 4217), for providers whose setup
    /// intent requires one even for a 0-amount card save (Stancer); others ignore it.
    pub currency: Option<String>,
}

/// Context for a combined mandate + first-payment hosted checkout. Threaded into
/// the provider's Billing Request so the single hosted flow both sets up the
/// reusable mandate and collects the first payment.
#[derive(Debug, Clone)]
pub struct HostedCheckoutContext {
    pub tenant_id: String,
    /// base62 `CheckoutSessionId` — recovered on `billing_requests.fulfilled` to
    /// activate the subscription in-flight. Goes in the BR/mandate metadata.
    pub checkout_session_id: String,
    /// base62 `PaymentTransactionId` of the local Pending checkout transaction —
    /// goes in the `payment_request` metadata so `payments.*` webhooks resolve
    /// straight to it.
    pub transaction_id: String,
    /// First-payment amount in minor units.
    pub amount_minor: i64,
    /// ISO 4217 currency of the first payment. MUST equal the mandate scheme's
    /// currency (GoCardless collects the payment in the scheme currency); the
    /// adapter rejects a mismatch so we never collect the wrong currency.
    pub currency: String,
}

/// Context for an in-flow hosted INVOICE payment, threaded into the provider
/// intent so completion resolves both the invoice and the pre-created Pending
/// transaction the capture must be recorded onto.
#[derive(Debug, Clone)]
pub struct HostedInvoicePaymentContext {
    /// base62 `InvoiceId` — goes in the intent metadata (`meteroid.invoice_id`).
    pub invoice_id: String,
    /// base62 id of the pre-created Pending transaction
    /// (`meteroid.transaction_id`): the capture is recorded onto THIS row.
    pub transaction_id: String,
    /// The invoice's `amount_due` in minor units, frozen at initiation.
    pub amount_minor: i64,
    /// ISO 4217 currency of the invoice.
    pub currency: String,
}

#[derive(Debug, Clone)]
pub struct CreateCustomerRequest {
    pub idempotency_key: IdempotencyKey,
}

/// Authoritative transaction status from the provider, used by reconciliation
/// when checking a stuck `Pending` transaction.
#[derive(Debug, Clone)]
pub enum RemoteTransactionStatus {
    Succeeded {
        amount_received_minor: i64,
        /// ISO 4217 currency the provider reports the settled amount in;
        /// cross-checked against the local transaction before settling.
        currency: String,
        processed_at: NaiveDateTime,
    },
    Pending,
    Failed {
        code: Option<String>,
        message: String,
        decline_kind: DeclineKind,
    },
    Cancelled,
    /// Provider has no record (our outbound call never reached it); safe to
    /// cancel the local transaction and retry from scratch.
    Unknown,
}
