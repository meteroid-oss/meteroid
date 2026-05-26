use super::error::ConnectorError;
use super::events::{NormalizedEventSubscription, NormalizedWebhookEvent};
use super::model::{
    ChargeOutcome, ChargeRequest, CreateCustomerRequest, ExternalCustomerRef,
    MandateSetupInstruction, MandateSetupRequest, PaymentMethodSnapshot, RefundOutcome,
    RefundRequest, RegisteredWebhook, RemoteTransactionStatus,
};
use crate::domain::connectors::Connector;
use crate::domain::enums::ConnectorProviderEnum;
use crate::domain::{Customer, CustomerConnection, PaymentMethodTypeEnum};
use async_trait::async_trait;
use error_stack::Report;
use http::HeaderMap;
use secrecy::SecretString;

/// Static description of what a connector can do. Used by:
/// - the core code to decide which operations to attempt
/// - the connector-config UI to render the right onboarding flow
/// - the contract test harness to skip tests irrelevant for this connector
#[derive(Debug, Clone)]
pub struct ConnectorCapabilities {
    pub supports_cards: bool,
    pub supports_mandates: bool,
    pub supports_refunds: bool,
    pub supports_partial_refunds: bool,
    pub supports_3ds: bool,
    pub supports_disputes: bool,
    pub supports_self_webhook_registration: bool,
    /// True if the provider asynchronously confirms charges via webhook
    /// (GoCardless, Stripe ACH, BACS). When true, `PaymentOps::charge_off_session`
    /// will frequently return `ChargeOutcome::Pending`.
    pub asynchronous_settlement: bool,
    pub supported_payment_methods: &'static [PaymentMethodTypeEnum],
    pub mandate_setup_mode: MandateSetupMode,
    /// Maximum age of a webhook signature we will accept, in seconds. Older
    /// payloads are rejected as replay attempts. Stripe recommends 300s.
    pub webhook_replay_tolerance_secs: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MandateSetupMode {
    /// Provider returns a `client_secret`; frontend mounts an embedded SDK.
    EmbeddedClientSecret,
    /// Provider returns an `authorisation_url`; frontend redirects the browser.
    HostedRedirect,
    /// Provider returns opaque session data; frontend mounts a Drop-in widget.
    EmbeddedDropIn,
}

/// Identifies a connector. The trait stays object-safe, so dynamic dispatch
/// (`Box<dyn PaymentConnector>`) is the intended use.
pub trait ConnectorIdentity: Send + Sync {
    fn provider(&self) -> ConnectorProviderEnum;
    fn capabilities(&self) -> &ConnectorCapabilities;
}

/// Customer-side operations.
///
/// **Retry semantics.** All methods carry an idempotency key derived from a
/// stable internal id; callers may retry on `ConnectorError::Transport` with
/// the exact same request and receive either the original response or a
/// provider-side dedup hit.
#[async_trait]
pub trait CustomerOps: Send + Sync {
    async fn create_customer(
        &self,
        connector: &Connector,
        customer: &Customer,
        request: CreateCustomerRequest,
    ) -> Result<ExternalCustomerRef, Report<ConnectorError>>;
}

/// Mandate / payment-method-setup operations.
#[async_trait]
pub trait MandateOps: Send + Sync {
    /// Starts the mandate / payment-method setup. The returned
    /// [`MandateSetupInstruction`] tells the frontend how to present the next
    /// step (embedded SDK, hosted redirect, drop-in).
    async fn initiate_mandate_setup(
        &self,
        connector: &Connector,
        connection: &CustomerConnection,
        request: MandateSetupRequest<'_>,
    ) -> Result<MandateSetupInstruction, Report<ConnectorError>>;

    /// Fetches the canonical snapshot of a payment method from the provider.
    /// Called after a `PaymentMethodAttached` webhook fires (or eagerly if the
    /// provider doesn't include enough detail in its webhook).
    async fn fetch_payment_method(
        &self,
        connector: &Connector,
        external_payment_method_id: &str,
        external_customer_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>>;

    /// Finalize a mandate / payment-method setup that requires a server-side
    /// completion step. Used by:
    ///
    /// - **HostedRedirect** providers (GoCardless): the customer returns from
    ///   the authorisation URL; we call this to complete the Billing Request
    ///   and extract the resulting mandate.
    /// - **EmbeddedDropIn** providers (Adyen): the client sends back the
    ///   drop-in payload; we call this to finalize.
    ///
    /// **EmbeddedClientSecret** providers (Stripe) don't need this — the SDK
    /// completes client-side and the provider's webhook is the authoritative
    /// signal. They return [`ConnectorError::Unsupported`].
    ///
    /// `intent_id` is the value returned by `initiate_mandate_setup` —
    /// `BillingRequest.id` for GoCardless, `SetupIntent.id` for Stripe.
    async fn complete_mandate_setup(
        &self,
        connector: &Connector,
        intent_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>>;
}

/// Off-session payment operations.
///
/// `Ok(ChargeOutcome::Failed(_))` means "the provider acknowledged the request
/// and refused it" — terminal, do not retry.
/// `Err(_)` means "we don't know what happened" (timeout, 5xx) — caller may
/// retry with the same `idempotency_key`; provider will dedup or complete.
#[async_trait]
pub trait PaymentOps: Send + Sync {
    async fn charge_off_session(
        &self,
        connector: &Connector,
        request: ChargeRequest<'_>,
    ) -> Result<ChargeOutcome, Report<ConnectorError>>;
}

/// Reconciliation operations.
///
/// Used by the background worker that checks on transactions stuck in
/// `Pending` past a threshold. Required because webhook delivery is best-effort:
/// providers retry but the guarantee is not absolute, and our outbound call may
/// have timed out before we recorded the external id. Polling the provider
/// directly is the safety net.
#[async_trait]
pub trait ReconcileOps: Send + Sync {
    /// Fetch the current status of a charge by its provider-side external id.
    /// Returns [`RemoteTransactionStatus::Unknown`] if the provider has no
    /// record of the id (typical when our outbound POST never reached them).
    async fn fetch_transaction_status(
        &self,
        connector: &Connector,
        external_transaction_id: &str,
    ) -> Result<RemoteTransactionStatus, Report<ConnectorError>>;
}

#[async_trait]
pub trait RefundOps: Send + Sync {
    async fn refund(
        &self,
        connector: &Connector,
        request: RefundRequest<'_>,
    ) -> Result<RefundOutcome, Report<ConnectorError>>;
}

/// Webhook-side operations: lifecycle management of the provider endpoint,
/// plus signature verification and event parsing.
///
/// **Verification contract.** `verify_signature` must reject:
/// 1. Payloads with a missing or malformed signature header.
/// 2. Payloads whose signature does not validate against `secret`.
/// 3. Payloads whose timestamp (when the provider's scheme includes one) is
///    outside `capabilities().webhook_replay_tolerance_secs` from now.
///
/// **Parsing contract.** `parse_event` runs *after* successful verification.
/// It must return `Ok(None)` for events the adapter recognizes but the core
/// doesn't act on (so they can still be logged for forensics).
#[async_trait]
pub trait WebhookOps: Send + Sync {
    /// Programmatically create a webhook endpoint on the provider side.
    /// Only callable when `capabilities().supports_self_webhook_registration`.
    /// Returns [`ConnectorError::Unsupported`] for providers without API
    /// support (GoCardless: dashboard-only).
    async fn register_webhook(
        &self,
        connector: &Connector,
        url: &str,
        subscriptions: &[NormalizedEventSubscription],
    ) -> Result<RegisteredWebhook, Report<ConnectorError>>;

    async fn unregister_webhook(
        &self,
        connector: &Connector,
        endpoint_id: &str,
    ) -> Result<(), Report<ConnectorError>>;

    /// Update the event subscription set on an existing endpoint. Called when
    /// Meteroid adds new event handlers and existing endpoints must start
    /// receiving them too.
    async fn sync_webhook_events(
        &self,
        connector: &Connector,
        endpoint_id: &str,
        subscriptions: &[NormalizedEventSubscription],
    ) -> Result<(), Report<ConnectorError>>;

    fn verify_signature(
        &self,
        connector: &Connector,
        payload: &[u8],
        headers: &HeaderMap,
        secret: &SecretString,
    ) -> Result<(), Report<ConnectorError>>;

    fn parse_event(
        &self,
        connector: &Connector,
        payload: &[u8],
        headers: &HeaderMap,
    ) -> Result<Option<NormalizedWebhookEvent>, Report<ConnectorError>>;

    /// Parse all events in one delivery (GoCardless batches; dropping any loses
    /// it once we ACK 200). The router uses this; the default suits one-event
    /// providers (Stripe), batching providers override.
    fn parse_events(
        &self,
        connector: &Connector,
        payload: &[u8],
        headers: &HeaderMap,
    ) -> Result<Vec<NormalizedWebhookEvent>, Report<ConnectorError>> {
        Ok(self
            .parse_event(connector, payload, headers)?
            .into_iter()
            .collect())
    }
}

/// Umbrella trait for payment connectors.
///
/// A concrete connector (e.g. `StripeConnector`, `GoCardlessConnector`)
/// implements every sub-trait. Methods on sub-traits the connector doesn't
/// support return [`ConnectorError::Unsupported`] rather than panic.
///
/// ## Retry contract
///
/// Connector implementations are "dumb": each method makes **one attempt**
/// against the provider and either returns the result or a
/// `ConnectorError::Transport`. They do not retry internally.
///
/// Retry policy (exponential backoff with jitter, capped at N attempts) is
/// applied by the caller via shared middleware. This keeps the policy uniform
/// across providers and avoids hidden retry storms.
///
/// Errors that are *not* `Transport` are not retryable — they indicate a
/// programming bug (`Configuration`, `InvalidMetadata`), a permanent
/// rejection (`Unsupported`), or a logical failure surfaced as an `Outcome`.
pub trait PaymentConnector:
    ConnectorIdentity
    + CustomerOps
    + MandateOps
    + PaymentOps
    + RefundOps
    + ReconcileOps
    + WebhookOps
{
}

impl<T> PaymentConnector for T where
    T: ConnectorIdentity
        + CustomerOps
        + MandateOps
        + PaymentOps
        + RefundOps
        + ReconcileOps
        + WebhookOps
{
}
