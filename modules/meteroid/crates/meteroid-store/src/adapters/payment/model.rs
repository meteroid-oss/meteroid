use crate::domain::PaymentMethodTypeEnum;
pub use crate::domain::enums::DeclineKind;
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
    /// Provider id of the underlying card/bank account, stable across re-adds (Stripe
    /// `fingerprint`, Mollie `cardFingerprint`). `None` for GoCardless and Stancer.
    pub fingerprint: Option<String>,
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
    /// Customer-facing payment description (Mollie).
    pub descriptor: Option<PaymentDescriptor>,
    /// Per-payment callback URL (Mollie); see [`connector_webhook_url`].
    pub webhook_url: Option<String>,
}

/// Phase 1 of a wallet top-up: the provider intent is created WITHOUT being confirmed, so its id
/// is persisted before anything can charge. A lost reply here charges nothing.
#[derive(Debug, Clone)]
pub struct PrepareWalletChargeRequest<'a> {
    pub transaction_id: PaymentTransactionId,
    pub customer_external_id: &'a str,
    pub payment_method_external_id: &'a str,
    pub payment_method_type: PaymentMethodTypeEnum,
    pub amount_minor: i64,
    pub currency: &'a str,
    /// Protects only the create call; the charge identity is the pinned intent id.
    pub idempotency_key: IdempotencyKey,
}

/// Where the provider sends this connector's webhooks, per its [`ConnectorWebhookRouteEnum`];
/// `None` for connectors that receive none.
///
/// [`ConnectorWebhookRouteEnum`]: crate::domain::enums::ConnectorWebhookRouteEnum
pub fn connector_webhook_url(
    webhook_base_url: &str,
    connector: &crate::domain::connectors::Connector,
) -> Option<String> {
    use crate::domain::enums::ConnectorWebhookRouteEnum;
    use common_domain::ids::BaseId;
    let base = webhook_base_url.trim_end_matches('/');
    connector.webhook_route.map(|route| match route {
        ConnectorWebhookRouteEnum::ConnectorId => {
            format!("{base}/webhooks/v1/payments/{}", connector.id.as_base62())
        }
        ConnectorWebhookRouteEnum::TenantAlias => format!(
            "{base}/webhooks/v1/{}/{}",
            connector.tenant_id.as_base62(),
            urlencoding::encode(&connector.alias)
        ),
    })
}

/// Customer-facing context of a payment: who pays and for what.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaymentDescriptor {
    /// The merchant's legal name (invoicing entity).
    pub merchant_name: String,
    pub invoice_number: Option<String>,
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

#[derive(Debug, Clone)]
pub struct RefundRequest<'a> {
    pub external_transaction_id: &'a str,
    pub amount_minor: i64,
    pub currency: &'a str,
    pub reason: Option<RefundReason>,
    /// Derived from the refund row id so retries reuse the same key.
    pub idempotency_key: IdempotencyKey,
    /// Correlation stamped on the provider refund object (`meteroid.refund_id`,
    /// `meteroid.tenant_id`, `meteroid.transaction_id`), the way charges stamp theirs.
    /// Providers without a metadata bag ignore it.
    pub metadata: Vec<(String, String)>,
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

/// A refund as the provider currently sees it. Resolves a webhook that carried no amounts
/// (GoCardless refund events only link the refund id) and backs the reconcile sweep's second
/// pass over Pending refund rows.
#[derive(Debug, Clone)]
pub struct RefundSnapshot {
    /// Provider id of the parent payment/charge.
    pub external_transaction_id: String,
    pub external_refund_id: String,
    pub amount_minor: i64,
    pub status: RemoteRefundStatus,
    /// The parent payment's cumulative refunded total, when the provider reports one;
    /// `None` when only this refund can be resolved (Stripe).
    pub cumulative_refunded_minor: Option<i64>,
    pub currency: String,
}

/// Authoritative refund status at the provider. `Canceled` is a refund the provider gave up on
/// (funds never left); the row is Failed either way, `Canceled` just carries no error text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteRefundStatus {
    Pending,
    Succeeded,
    Failed {
        code: Option<String>,
        message: String,
    },
    Canceled,
}

/// `secret` is the signing secret for future events; we persist it on the connector.
#[derive(Debug, Clone)]
pub struct RegisteredWebhook {
    pub endpoint_id: String,
    pub secret: SecretString,
}

/// One inbound webhook to archive and enqueue: its dedup key (provider event id, `None` for
/// payloadless notifications) and the body the worker parses.
#[derive(Debug, Clone)]
pub struct WebhookDeliveryUnit {
    pub event_id: Option<String>,
    pub body: Vec<u8>,
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
    /// What the hosted payment is for (see [`ChargeRequest::descriptor`]).
    pub descriptor: Option<PaymentDescriptor>,
    /// See [`ChargeRequest::webhook_url`].
    pub webhook_url: Option<String>,
    /// Stamped on an embedded setup intent so its webhook knows what the method is for.
    pub purpose: SetupPurpose,
}

/// Why the customer sets up a method: to pay with it right away, or on its own (portal settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupPurpose {
    Payment,
    Settings,
}

impl SetupPurpose {
    pub const METADATA_KEY: &'static str = "meteroid.setup_purpose";

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Payment => "payment",
            Self::Settings => "settings",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "payment" => Some(Self::Payment),
            "settings" => Some(Self::Settings),
            _ => None,
        }
    }
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
    /// Pending, possibly on the customer (3DS, a redirect, a mandate approval).
    Pending,
    /// Pending on the provider alone (a debit clearing): no customer action is awaited.
    Processing,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::connectors::{ConnectorConfig, CredentialState, PaymentConnectorConfig};
    use common_domain::ids::{BaseId, TenantId};

    #[test]
    fn connector_webhook_url_uses_the_id_unless_legacy() {
        use crate::domain::connectors::Connector;
        use crate::domain::enums::{ConnectorStatusEnum, ConnectorWebhookRouteEnum};
        use common_domain::ids::ConnectorId;

        let mut connector = Connector {
            id: ConnectorId::new(),
            created_at: common_domain::clock::now().naive_utc(),
            tenant_id: TenantId::new(),
            alias: "mollie eu".to_string(),
            config: ConnectorConfig::Payment(PaymentConnectorConfig::draft(
                crate::domain::enums::PaymentProvider::Mollie,
            )),
            status: ConnectorStatusEnum::Active,
            credentials: CredentialState::Unverified,
            webhook_route: Some(ConnectorWebhookRouteEnum::ConnectorId),
        };
        assert_eq!(
            connector_webhook_url("https://api.example.com/", &connector),
            Some(format!(
                "https://api.example.com/webhooks/v1/payments/{}",
                connector.id.as_base62()
            ))
        );
        connector.webhook_route = Some(ConnectorWebhookRouteEnum::TenantAlias);
        assert_eq!(
            connector_webhook_url("https://api.example.com/", &connector),
            Some(format!(
                "https://api.example.com/webhooks/v1/{}/mollie%20eu",
                connector.tenant_id.as_base62()
            ))
        );
        connector.webhook_route = None;
        assert_eq!(
            connector_webhook_url("https://api.example.com/", &connector),
            None
        );
    }
}
