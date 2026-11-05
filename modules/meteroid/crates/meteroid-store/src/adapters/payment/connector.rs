use super::account::ValidatedCredentials;
use super::error::ConnectorError;
use super::events::{NormalizedEventSubscription, NormalizedWebhookEvent};
use super::model::PrepareWalletChargeRequest;
use super::model::{
    ChargeOutcome, ChargeRequest, CreateCustomerRequest, ExternalCustomerRef,
    MandateSetupInstruction, MandateSetupRequest, PaymentMethodSnapshot, RefundOutcome,
    RefundRequest, RefundSnapshot, RegisteredWebhook, RemoteTransactionStatus,
    RequiresActionInstruction, WebhookDeliveryUnit,
};
use crate::domain::connectors::Connector;
use crate::domain::enums::PaymentProvider;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::{Customer, CustomerConnection, PaymentMethodTypeEnum};
use async_trait::async_trait;
use common_domain::ids::{ConnectorId, CustomerPaymentMethodId, InvoiceId, TenantId};
use error_stack::Report;
use http::HeaderMap;
use secrecy::SecretString;
use std::num::NonZeroU32;
use std::time::Duration;

/// The connector an adapter is bound to: what it needs about the connection beyond its
/// credentials. An adapter is built for one connector and holds this for its whole life.
#[derive(Clone, Debug)]
pub struct ConnectorRef {
    pub id: ConnectorId,
    pub tenant_id: TenantId,
    /// Names the connection in provider-side descriptions and legacy webhook URLs.
    pub alias: String,
}

impl From<&Connector> for ConnectorRef {
    fn from(connector: &Connector) -> Self {
        Self {
            id: connector.id,
            tenant_id: connector.tenant_id,
            alias: connector.alias.clone(),
        }
    }
}

/// Raw query string, injected by the webhook router. Lets unsigned webhooks (Mollie) authenticate
/// with a per-connector URL token.
pub const REQUEST_QUERY_HEADER: &str = "x-meteroid-request-query";

/// Static description of what a payment provider can do. Drives operation selection,
/// onboarding UI, and which contract tests apply.
#[derive(Debug, Clone)]
pub struct PaymentCapabilities {
    /// Card support is not a separate flag: it is whether `Card` is in here.
    pub supported_payment_methods: &'static [PaymentMethodTypeEnum],
    pub supports_mandates: bool,
    pub refunds: RefundSupport,
    /// How long the provider dedups a refund replayed with the same idempotency key. An
    /// unconfirmed refund is re-issued by the sweep only inside it; past it, or with `None`, a
    /// replay could move the money twice.
    pub refund_replay_window: Option<Duration>,
    /// How long a charge re-sent under its idempotency key is deduplicated (a Stancer
    /// `unique_id` is looked up instead). An attempt whose outcome never came back is re-sent
    /// only inside it; past it, a re-send could charge twice, so the attempt is failed.
    pub charge_replay_window: Duration,
    pub supports_3ds: bool,
    pub supports_disputes: bool,
    /// On what terms inbound webhooks arrive; `None` for a provider that sends none (Stancer),
    /// whose outcomes are polled instead.
    pub webhooks: Option<InboundWebhooks>,
    /// The outcome isn't known when the charge returns, so `charge_off_session` frequently
    /// answers `ChargeOutcome::Pending`. Independent of `webhooks`: the outcome arrives by
    /// webhook where the provider sends them, and by the reconciliation sweep where it doesn't
    /// (Stancer).
    pub asynchronous_settlement: bool,
    /// How a mandate is set up, and — for hosted redirects — how its completion reaches us.
    pub mandate_setup: MandateSetup,
    /// The checkout's first payment and the mandate are collected in one hosted flow.
    pub supports_hosted_checkout: bool,
    /// A `Pending` charge with no next action is in flight on every rail (cards included), not
    /// waiting on the customer.
    pub pending_charge_accepted: bool,
    /// Simulated outcomes, no money moves: the frontend picks the scenario UI from this.
    pub is_test_gateway: bool,
}

/// What a provider can reverse. Partial refunds imply refunds, so the two can't disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefundSupport {
    None,
    /// Whole payments only.
    Full,
    Partial,
}

/// What verifying a provider's inbound webhooks needs. A provider that sends none has no secret
/// and no replay window to get wrong, so it holds no value of this type at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboundWebhooks {
    /// The connector must hold a signing secret (pasted, or stored by self-registration) before
    /// it can go live. Mollie's URL token is generated for us, so it needs none.
    pub requires_secret: bool,
    /// The provider exposes an API to register our endpoint (Stripe); otherwise the merchant
    /// configures it in the provider's dashboard.
    pub self_registration: bool,
    /// Max webhook signature age accepted; older payloads are rejected as replays. Non-zero,
    /// because a zero window would reject every webhook.
    pub replay_tolerance_secs: NonZeroU32,
}

/// How a mandate is set up. `HostedRedirect` is the only flow whose completion arrives out of
/// band, so it is the only variant that carries completion details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MandateSetup {
    /// Provider returns a `client_secret`; frontend mounts an embedded SDK.
    EmbeddedClientSecret,
    /// Provider returns opaque session data; frontend mounts a Drop-in widget.
    EmbeddedDropIn,
    /// Provider returns an `authorisation_url`; frontend redirects the browser.
    HostedRedirect {
        completion: HostedSetupCompletion,
        /// Collects the invoice on the provider's hosted page; completion records it, never
        /// recharges.
        invoice_payment: bool,
    },
}

/// Return target of hosted-redirect providers (one provider-agnostic handler).
pub const HOSTED_RETURN_PATH: &str = "v1/portal/hosted/return";

impl PaymentCapabilities {
    pub fn supports_cards(&self) -> bool {
        self.supported_payment_methods
            .contains(&PaymentMethodTypeEnum::Card)
    }

    pub fn supports_refunds(&self) -> bool {
        self.refunds != RefundSupport::None
    }

    pub fn supports_partial_refunds(&self) -> bool {
        self.refunds == RefundSupport::Partial
    }

    pub fn receives_webhooks(&self) -> bool {
        self.webhooks.is_some()
    }

    pub fn requires_webhook_secret(&self) -> bool {
        self.webhooks.is_some_and(|w| w.requires_secret)
    }

    pub fn supports_self_webhook_registration(&self) -> bool {
        self.webhooks.is_some_and(|w| w.self_registration)
    }

    /// `None` for a provider that sends no webhooks.
    pub fn webhook_replay_tolerance_secs(&self) -> Option<u32> {
        self.webhooks.map(|w| w.replay_tolerance_secs.get())
    }

    /// How the frontend presents the setup step.
    pub fn mandate_setup_mode(&self) -> MandateSetupMode {
        match self.mandate_setup {
            MandateSetup::EmbeddedClientSecret => MandateSetupMode::EmbeddedClientSecret,
            MandateSetup::EmbeddedDropIn => MandateSetupMode::EmbeddedDropIn,
            MandateSetup::HostedRedirect { .. } => MandateSetupMode::HostedRedirect,
        }
    }

    /// `None` unless the setup is a hosted redirect, the only flow with out-of-band completion.
    pub fn hosted_setup_completion(&self) -> Option<HostedSetupCompletion> {
        match self.mandate_setup {
            MandateSetup::HostedRedirect { completion, .. } => Some(completion),
            _ => None,
        }
    }

    /// The provider's own event closes a hosted setup, so a lost return redirect is not fatal.
    pub fn is_webhook_backed_hosted_setup(&self) -> bool {
        self.hosted_setup_completion() == Some(HostedSetupCompletion::WebhookBacked)
    }

    pub fn supports_hosted_invoice_payment(&self) -> bool {
        matches!(
            self.mandate_setup,
            MandateSetup::HostedRedirect {
                invoice_payment: true,
                ..
            }
        )
    }

    /// The intent id is persisted; the return handler or the sweeper completes it.
    pub fn completes_pending_hosted_intents(&self) -> bool {
        match self.mandate_setup {
            MandateSetup::HostedRedirect {
                completion,
                invoice_payment,
            } => completion == HostedSetupCompletion::PollingRequired || invoice_payment,
            _ => false,
        }
    }

    pub fn is_hosted_redirect(&self) -> bool {
        matches!(self.mandate_setup, MandateSetup::HostedRedirect { .. })
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
    fn provider(&self) -> PaymentProvider;
    fn capabilities(&self) -> &PaymentCapabilities;
}

/// Runs when credentials are set or rotated, and periodically to refresh the account.
#[async_trait]
pub trait CredentialOps: Send + Sync {
    /// Checks the credentials with the provider and returns the public data to persist (e.g. the
    /// Stripe account id) plus the account and the payment methods it has enabled.
    /// `Configuration` = rejected, `Transport` = provider unreachable.
    async fn validate_credentials(&self) -> Result<ValidatedCredentials, Report<ConnectorError>>;
}

/// All methods carry an idempotency key from a stable internal id; callers may
/// retry on `ConnectorError::Transport` with the same request and get the
/// original response or a provider-side dedup hit.
#[async_trait]
pub trait CustomerOps: Send + Sync {
    async fn create_customer(
        &self,
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
        connection: &CustomerConnection,
        request: MandateSetupRequest<'_>,
    ) -> Result<MandateSetupInstruction, Report<ConnectorError>>;

    /// Called after a `PaymentMethodAttached` webhook fires (or eagerly when the
    /// webhook lacks enough detail).
    async fn fetch_payment_method(
        &self,
        external_payment_method_id: &str,
        external_customer_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>>;

    /// Server-side completion step for HostedRedirect/EmbeddedDropIn providers.
    /// EmbeddedClientSecret providers (Stripe) complete client-side and return
    /// `Unsupported`. `intent_id` is the value from `initiate_mandate_setup`.
    async fn complete_mandate_setup(
        &self,
        intent_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>>;

    /// Cancel a previously issued setup intent so its hosted flow can never
    /// complete or collect money. `Ok(())` MUST mean the intent is dead at the
    /// provider; an intent that may still capture — or already captured —
    /// money MUST be `Err`. Default: no-op for providers with a webhook
    /// backstop or client-side completion.
    async fn cancel_mandate_setup(&self, _intent_id: &str) -> Result<(), Report<ConnectorError>> {
        Ok(())
    }
}

/// `Ok(ChargeOutcome::Failed)` is terminal (provider refused). `Err` is unknown
/// (timeout, 5xx) and retryable with the same `idempotency_key`.
#[async_trait]
pub trait PaymentOps: Send + Sync {
    async fn charge_off_session(
        &self,
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

    /// Re-reads a charge left awaiting customer authentication (3DS / SCA), so the customer can
    /// resume it on-session: the client secret is never stored. `Ok(None)` when the provider no
    /// longer awaits an action on it, or cannot resume one.
    async fn fetch_pending_action(
        &self,
        _external_transaction_id: &str,
    ) -> Result<Option<RequiresActionInstruction>, Report<ConnectorError>> {
        Ok(None)
    }
}

/// Wallet top-ups in two phases: create an UNCONFIRMED intent, persist its id, then confirm that
/// same id. The pinned id, not an idempotency key, is what makes a second successful charge
/// impossible. Providers without unconfirmed intents keep the `Unsupported` defaults.
#[async_trait]
pub trait WalletOps: ConnectorIdentity {
    async fn prepare_wallet_charge(
        &self,
        _request: PrepareWalletChargeRequest<'_>,
    ) -> Result<String, Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: self.provider().into(),
            capability: "wallet.prepare_charge",
        }))
    }

    /// Advances the intent pinned on `transaction.provider_transaction_id`; never creates one, and
    /// cancels an intent that cannot continue so no exposed client secret charges later.
    async fn resume_wallet_charge(
        &self,
        _transaction: &PaymentTransaction,
        _on_session: bool,
    ) -> Result<ChargeOutcome, Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: self.provider().into(),
            capability: "wallet.resume_charge",
        }))
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
        external_transaction_id: &str,
    ) -> Result<RemoteTransactionStatus, Report<ConnectorError>>;
}

#[async_trait]
pub trait RefundOps: Send + Sync {
    async fn refund(
        &self,
        request: RefundRequest<'_>,
    ) -> Result<RefundOutcome, Report<ConnectorError>>;

    /// Resolve a refund at the provider. Two callers: the webhook path, for events that
    /// carry no amounts (`NormalizedEventKind::RefundObserved`, GoCardless), and the
    /// reconcile sweep, which polls a REFUND row still Pending to find out what really
    /// happened. Providers with no refund GET return `Unsupported`; the sweep skips them.
    /// `external_transaction_id` is the parent payment when the caller knows it: Mollie only
    /// reads a refund through its payment.
    async fn fetch_refund(
        &self,
        external_refund_id: &str,
        external_transaction_id: Option<&str>,
    ) -> Result<RefundSnapshot, Report<ConnectorError>>;
}

/// `verify_signature` must reject missing/malformed signatures, signatures that
/// don't validate against `secret`, and timestamps outside
/// `webhook_replay_tolerance_secs`. `parse_event` runs after verification and
/// returns `Ok(None)` for recognized-but-unhandled events (still logged).
#[async_trait]
pub trait WebhookOps: ConnectorIdentity {
    /// Only callable when `supports_self_webhook_registration`; otherwise
    /// returns `Unsupported` (GoCardless: dashboard-only).
    async fn register_webhook(
        &self,
        url: &str,
        subscriptions: &[NormalizedEventSubscription],
    ) -> Result<RegisteredWebhook, Report<ConnectorError>>;

    async fn unregister_webhook(&self, endpoint_id: &str) -> Result<(), Report<ConnectorError>>;

    /// Update the subscription set on an existing endpoint when new event
    /// handlers are added.
    async fn sync_webhook_events(
        &self,
        endpoint_id: &str,
        subscriptions: &[NormalizedEventSubscription],
    ) -> Result<(), Report<ConnectorError>>;

    /// Identifies what `sync_webhook_events` would register for `subscriptions`; an endpoint
    /// whose stored fingerprint matches is already up to date. Providers that map subscriptions
    /// to concrete event names override this so a new event name changes the fingerprint.
    fn webhook_subscription_fingerprint(
        &self,
        subscriptions: &[NormalizedEventSubscription],
    ) -> String {
        subscription_fingerprint(
            &subscriptions
                .iter()
                .map(|s| format!("{s:?}"))
                .collect::<Vec<_>>(),
        )
    }

    fn verify_signature(
        &self,
        payload: &[u8],
        headers: &HeaderMap,
        secret: &SecretString,
    ) -> Result<(), Report<ConnectorError>>;

    fn parse_event(
        &self,
        payload: &[u8],
        headers: &HeaderMap,
    ) -> Result<Option<NormalizedWebhookEvent>, Report<ConnectorError>>;

    /// Turns a `ResourceChanged` notification into concrete events by re-reading the resource.
    /// Must not return another `ResourceChanged`.
    async fn resolve_resource_change(
        &self,
        _resource_ref: &str,
    ) -> Result<Vec<NormalizedWebhookEvent>, Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: self.provider().into(),
            capability: "webhook.resolve_resource_change",
        }))
    }

    /// Splits a verified delivery into units, each keyed by provider event id for dedup.
    /// Default: one JSON body keyed by its top-level `id`.
    fn split_delivery(
        &self,
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
        payload: &[u8],
        headers: &HeaderMap,
    ) -> Result<Vec<NormalizedWebhookEvent>, Report<ConnectorError>> {
        Ok(self.parse_event(payload, headers)?.into_iter().collect())
    }
}

/// Stable across processes AND across toolchains: std's default hasher leaves its algorithm
/// unspecified, so a compiler bump would invalidate every stored fingerprint at once and re-push
/// an unchanged event set to every connector.
pub fn subscription_fingerprint(values: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut sorted: Vec<&str> = values.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    let digest = Sha256::digest(sorted.join("\n").as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
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
    + WalletOps
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
        + WalletOps
        + RefundOps
        + ReconcileOps
        + WebhookOps
{
}
