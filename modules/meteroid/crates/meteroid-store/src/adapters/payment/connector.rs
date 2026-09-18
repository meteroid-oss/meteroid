use super::error::ConnectorError;
use super::events::{NormalizedEventSubscription, NormalizedWebhookEvent};
use super::model::{
    ChargeOutcome, ChargeRequest, CreateCustomerRequest, ExternalCustomerRef,
    MandateSetupInstruction, MandateSetupRequest, PaymentMethodSnapshot, RefundOutcome,
    RefundRequest, RefundSnapshot, RegisteredWebhook, RemoteTransactionStatus, WebhookDeliveryUnit,
};
use crate::domain::connectors::{Connector, ProviderData};
use crate::domain::enums::ConnectorProviderEnum;
use crate::domain::{Customer, CustomerConnection, PaymentMethodTypeEnum};
use async_trait::async_trait;
use common_domain::ids::{CustomerPaymentMethodId, InvoiceId};
use error_stack::Report;
use http::HeaderMap;
use secrecy::SecretString;

/// Raw query string, injected by the webhook router. Lets unsigned webhooks (Mollie) authenticate
/// with a per-connector URL token.
pub const REQUEST_QUERY_HEADER: &str = "x-meteroid-request-query";

/// Static description of what a connector can do. Drives operation selection,
/// onboarding UI, and which contract tests apply.
#[derive(Debug, Clone)]
pub struct ConnectorCapabilities {
    pub supports_cards: bool,
    pub supports_mandates: bool,
    pub supports_refunds: bool,
    pub supports_partial_refunds: bool,
    pub supports_3ds: bool,
    pub supports_disputes: bool,
    pub supports_self_webhook_registration: bool,
    /// Provider confirms charges asynchronously via webhook; `charge_off_session`
    /// then frequently returns `ChargeOutcome::Pending`.
    pub asynchronous_settlement: bool,
    pub supported_payment_methods: &'static [PaymentMethodTypeEnum],
    pub mandate_setup_mode: MandateSetupMode,
    /// Max webhook signature age accepted, in seconds; older payloads are rejected as replays.
    pub webhook_replay_tolerance_secs: u32,
    /// How a hosted-redirect setup's completion reaches us. Drives whether the
    /// hosted-checkout pending-intent id is persisted and swept.
    pub hosted_setup_completion: HostedSetupCompletion,
    /// Collects the invoice on the provider's hosted page; completion records it, never recharges.
    pub supports_hosted_invoice_payment: bool,
    /// The checkout's first payment and the mandate are collected in one hosted flow.
    pub supports_hosted_checkout: bool,
    /// A `Pending` charge with no next action is in flight on every rail (cards included), not
    /// waiting on the customer.
    pub pending_charge_accepted: bool,
}

/// Return target of hosted-redirect providers (one provider-agnostic handler).
pub const HOSTED_RETURN_PATH: &str = "v1/portal/hosted/return";

impl ConnectorCapabilities {
    /// The intent id is persisted; the return handler or the sweeper completes it.
    pub fn completes_pending_hosted_intents(&self) -> bool {
        self.hosted_setup_completion == HostedSetupCompletion::PollingRequired
            || self.supports_hosted_invoice_payment
    }

    pub fn is_hosted_redirect(&self) -> bool {
        self.mandate_setup_mode == MandateSetupMode::HostedRedirect
    }

    pub fn supports_direct_debit(&self) -> bool {
        self.supported_payment_methods.iter().any(|m| {
            matches!(
                m,
                PaymentMethodTypeEnum::DirectDebitSepa
                    | PaymentMethodTypeEnum::DirectDebitAch
                    | PaymentMethodTypeEnum::DirectDebitBacs
            )
        })
    }
}

/// How the outcome of a hosted setup flow is delivered once the customer
/// finishes on the provider's hosted page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedSetupCompletion {
    /// A webhook covers lost returns, so checkout intents are not persisted or swept (invoice
    /// payments still are).
    WebhookBacked,
    /// No webhook exists: the return redirect is the only push signal, so the
    /// intent id is persisted and the sweeper polls it as the lost-return backstop.
    PollingRequired,
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

/// Object-safe; intended for dynamic dispatch (`Box<dyn PaymentConnector>`).
pub trait ConnectorIdentity: Send + Sync {
    fn provider(&self) -> ConnectorProviderEnum;
    fn capabilities(&self) -> &ConnectorCapabilities;
}

/// Runs once at connect time, before the connector is persisted.
#[async_trait]
pub trait CredentialOps: Send + Sync {
    /// Checks the credentials with the provider and returns the public data to persist (e.g. the
    /// Stripe account id). `Configuration` = rejected, `Transport` = provider unreachable.
    async fn validate_credentials(
        &self,
        connector: &Connector,
    ) -> Result<ProviderData, Report<ConnectorError>>;
}

/// All methods carry an idempotency key from a stable internal id; callers may
/// retry on `ConnectorError::Transport` with the same request and get the
/// original response or a provider-side dedup hit.
#[async_trait]
pub trait CustomerOps: Send + Sync {
    async fn create_customer(
        &self,
        connector: &Connector,
        customer: &Customer,
        request: CreateCustomerRequest,
    ) -> Result<ExternalCustomerRef, Report<ConnectorError>>;
}

#[async_trait]
pub trait MandateOps: Send + Sync {
    /// The returned [`MandateSetupInstruction`] tells the frontend how to present
    /// the next step (embedded SDK, hosted redirect, drop-in).
    async fn initiate_mandate_setup(
        &self,
        connector: &Connector,
        connection: &CustomerConnection,
        request: MandateSetupRequest<'_>,
    ) -> Result<MandateSetupInstruction, Report<ConnectorError>>;

    /// Called after a `PaymentMethodAttached` webhook fires (or eagerly when the
    /// webhook lacks enough detail).
    async fn fetch_payment_method(
        &self,
        connector: &Connector,
        external_payment_method_id: &str,
        external_customer_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>>;

    /// Server-side completion step for HostedRedirect/EmbeddedDropIn providers.
    /// EmbeddedClientSecret providers (Stripe) complete client-side and return
    /// `Unsupported`. `intent_id` is the value from `initiate_mandate_setup`.
    async fn complete_mandate_setup(
        &self,
        connector: &Connector,
        intent_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>>;

    /// Cancel a previously issued setup intent so its hosted flow can never
    /// complete or collect money. `Ok(())` MUST mean the intent is dead at the
    /// provider; an intent that may still capture — or already captured —
    /// money MUST be `Err`. Default: no-op for providers with a webhook
    /// backstop or client-side completion.
    async fn cancel_mandate_setup(
        &self,
        _connector: &Connector,
        _intent_id: &str,
    ) -> Result<(), Report<ConnectorError>> {
        Ok(())
    }
}

/// `Ok(ChargeOutcome::Failed)` is terminal (provider refused). `Err` is unknown
/// (timeout, 5xx) and retryable with the same `idempotency_key`.
#[async_trait]
pub trait PaymentOps: Send + Sync {
    async fn charge_off_session(
        &self,
        connector: &Connector,
        request: ChargeRequest<'_>,
    ) -> Result<ChargeOutcome, Report<ConnectorError>>;

    /// Idempotency seed built from committed state, so the key survives a rollback and retry.
    /// `None` uses the transaction id.
    fn invoice_charge_idempotency_seed(
        &self,
        _payment_method_id: &CustomerPaymentMethodId,
        _invoice_id: &InvoiceId,
        _prior_invoice_attempts: usize,
    ) -> Option<String> {
        None
    }
}

/// Safety net for the worker polling transactions stuck in `Pending`: webhook
/// delivery is best-effort and our outbound call may time out before we record
/// the external id.
#[async_trait]
pub trait ReconcileOps: Send + Sync {
    /// Returns [`RemoteTransactionStatus::Unknown`] when the provider has no
    /// record of the id (our outbound POST never reached them).
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

    /// Resolve a webhook-observed refund (`NormalizedEventKind::RefundObserved`)
    /// into its parent payment and cumulative refunded total. Only providers
    /// whose refund webhooks carry no amounts need this (GoCardless); providers
    /// that inline amounts on the event return `Unsupported`.
    async fn fetch_refund(
        &self,
        connector: &Connector,
        external_refund_id: &str,
    ) -> Result<RefundSnapshot, Report<ConnectorError>>;
}

/// `verify_signature` must reject missing/malformed signatures, signatures that
/// don't validate against `secret`, and timestamps outside
/// `webhook_replay_tolerance_secs`. `parse_event` runs after verification and
/// returns `Ok(None)` for recognized-but-unhandled events (still logged).
#[async_trait]
pub trait WebhookOps: Send + Sync {
    /// Only callable when `supports_self_webhook_registration`; otherwise
    /// returns `Unsupported` (GoCardless: dashboard-only).
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

    /// Update the subscription set on an existing endpoint when new event
    /// handlers are added.
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

    /// Turns a `ResourceChanged` notification into concrete events by re-reading the resource.
    /// Must not return another `ResourceChanged`.
    async fn resolve_resource_change(
        &self,
        connector: &Connector,
        _resource_ref: &str,
    ) -> Result<Vec<NormalizedWebhookEvent>, Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: connector.provider.clone(),
            capability: "webhook.resolve_resource_change",
        }))
    }

    /// Splits a verified delivery into units, each keyed by provider event id for dedup.
    /// Default: one JSON body keyed by its top-level `id`.
    fn split_delivery(
        &self,
        _connector: &Connector,
        payload: &[u8],
    ) -> Result<Vec<WebhookDeliveryUnit>, Report<ConnectorError>> {
        let json: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| Report::new(ConnectorError::PayloadDecode(e.to_string())))?;
        Ok(vec![WebhookDeliveryUnit {
            event_id: json.get("id").and_then(|v| v.as_str()).map(str::to_string),
            body: payload.to_vec(),
        }])
    }

    /// Parse all events in one delivery; dropping any loses it once we ACK 200.
    /// Default suits one-event providers (Stripe); batching providers (GoCardless) override.
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

/// A connector implements every sub-trait; unsupported methods return
/// `Unsupported` rather than panic.
///
/// Retry contract: each method makes one attempt and returns either the result
/// or `Transport`; the caller applies backoff via shared middleware. Only
/// `Transport` is retryable — other errors are bugs, permanent rejections, or
/// logical failures surfaced as an `Outcome`.
pub trait PaymentConnector:
    ConnectorIdentity
    + CredentialOps
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
        + CredentialOps
        + CustomerOps
        + MandateOps
        + PaymentOps
        + RefundOps
        + ReconcileOps
        + WebhookOps
{
}
