//! Stripe implementation of the [`super::PaymentConnector`] trait family.
//! All Stripe-specific type mapping lives here; the rest of the codebase only
//! sees the normalized types from [`super::model`] and [`super::events`].

use super::account::{ValidatedCredentials, mask_secret};
use super::connector::{
    ConnectorIdentity, ConnectorRef, CredentialOps, CustomerOps, InboundWebhooks, MandateOps,
    MandateSetup, PaymentCapabilities, PaymentOps, ReconcileOps, RefundOps, RefundSupport,
    WebhookOps, subscription_fingerprint,
};
use super::error::{ConnectorError, CredentialsRejected};
use super::events::{
    DisputeEvent, NormalizedEventKind, NormalizedEventSubscription, NormalizedWebhookEvent,
    ObservedRefundOnWire, PaymentFailedEvent, PaymentMethodAttachedEvent,
    PaymentMethodDetachedEvent, PaymentMethodUpdatedEvent, PaymentPendingEvent,
    PaymentRefundedEvent, PaymentRequiresActionEvent, PaymentSucceededEvent,
    RefundStatusChangedEvent,
};
use super::model::{
    ChargeAcknowledged, ChargeCancelled, ChargeFailure, ChargeOutcome, ChargeReceipt,
    ChargeRequest, CreateCustomerRequest, DeclineKind, ExternalCustomerRef,
    MandateSetupInstruction, MandateSetupRequest, PaymentMethodSnapshot, RefundAcknowledged,
    RefundFailure, RefundOutcome, RefundReason, RefundReceipt, RefundRequest, RefundSnapshot,
    RegisteredWebhook, RemoteRefundStatus, RemoteTransactionStatus, RequiresActionInstruction,
    SetupPurpose,
};
use crate::domain::ObservedRefundStatus;
use crate::domain::connectors::{
    MethodState, PaymentConnectorPublicData, PaymentMethodAvailability, ProviderAccount,
    StripePublicData, StripeSensitiveData,
};
use crate::domain::enums::{PaymentProvider, ReversalKind};
use crate::domain::{Address, Customer, CustomerConnection, PaymentMethodTypeEnum};
use async_trait::async_trait;
use chrono::DateTime;
use common_domain::ids::BaseId;
use error_stack::Report;
use http::HeaderMap;
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::OnceLock;
use std::time::Duration;
use stripe_client::client::StripeClient;
use stripe_client::customers::{
    CreateCustomer, CustomerApi, CustomerShipping, OptionalFieldsAddress,
};
use stripe_client::error::StripeError;
use stripe_client::payment_intents::{
    PaymentIntentApi, PaymentIntentRequest, StripeNextAction, StripePaymentError,
    StripePaymentIntent, StripePaymentStatus,
};
use stripe_client::payment_method_configurations::PaymentMethodConfiguration;
use stripe_client::payment_methods::{
    PaymentMethod, PaymentMethodsApi, StripePaymentMethodType as StripePmType,
};
use stripe_client::refunds::{
    CreateRefund, RefundApi, StripeRefund, StripeRefundReason, refund_status,
};
use stripe_client::setup_intents::{
    CreateSetupIntent, CreateSetupIntentUsage, SetupIntentApi,
    StripePaymentMethodType as StripeSetupPmType,
};
use stripe_client::webhook::{Event, EventObject, StripeRefundList, StripeWebhook, event_type};
use stripe_client::webhook_endpoints::{
    CreateWebhookEndpointRequest, UpdateWebhookEndpointRequest, WebhookEndpointApi,
};

mod wallet;

/// The bits describe Stripe's *protocol* capability, gated by impl state: every
/// one advertised here is implemented, refunds and webhook self-registration
/// included.
pub(super) const STRIPE_CAPABILITIES: PaymentCapabilities = PaymentCapabilities {
    supported_payment_methods: &[
        PaymentMethodTypeEnum::Card,
        PaymentMethodTypeEnum::DirectDebitSepa,
        PaymentMethodTypeEnum::DirectDebitAch,
        PaymentMethodTypeEnum::DirectDebitBacs,
    ],
    supports_mandates: true,
    // `POST /v1/refunds` takes an explicit `amount`.
    refunds: RefundSupport::Partial,
    // Stripe keeps an Idempotency-Key 24h; replays stay well inside it.
    refund_replay_window: Some(Duration::from_secs(12 * 60 * 60)),
    charge_replay_window: Duration::from_secs(12 * 60 * 60),
    supports_3ds: true,
    supports_disputes: true,
    webhooks: Some(InboundWebhooks {
        requires_secret: true,
        // Implemented, and auto-invoked on connect.
        self_registration: true,
        replay_tolerance_secs: NonZeroU32::new(300).unwrap(),
    }),
    asynchronous_settlement: true,
    // Setup completes client-side and `payment_method.attached` /
    // `setup_intent.succeeded` webhooks back it up — never swept.
    mandate_setup: MandateSetup::EmbeddedClientSecret,
    supports_hosted_checkout: false,
    pending_charge_accepted: false,
    is_test_gateway: false,
};

/// Bound to one Stripe connector: holds its identity and typed credentials for
/// life. The [`StripeClient`] itself is process-wide — every tenant shares one
/// pool since they all hit `api.stripe.com`.
#[derive(Debug, Clone)]
pub struct StripeConnector {
    connector: ConnectorRef,
    public: StripePublicData,
    sensitive: StripeSensitiveData,
}

impl StripeConnector {
    pub fn new(
        connector: ConnectorRef,
        public: StripePublicData,
        sensitive: StripeSensitiveData,
    ) -> Self {
        Self {
            connector,
            public,
            sensitive,
        }
    }

    /// Process-wide pooled client; the wrapper holds tuning (timeouts, retry
    /// strategy) we want fixed at process startup.
    fn client() -> &'static StripeClient {
        static CLIENT: OnceLock<StripeClient> = OnceLock::new();
        CLIENT.get_or_init(StripeClient::new)
    }

    fn secret_key(&self) -> SecretString {
        self.sensitive.api_secret_key.clone()
    }

    /// The publishable key is public by definition; the client takes every key as a secret.
    fn publishable_key(&self) -> SecretString {
        SecretString::from(self.public.api_publishable_key.clone())
    }
}

impl ConnectorIdentity for StripeConnector {
    fn provider(&self) -> PaymentProvider {
        PaymentProvider::Stripe
    }

    fn capabilities(&self) -> &PaymentCapabilities {
        &STRIPE_CAPABILITIES
    }
}

#[async_trait]
impl CredentialOps for StripeConnector {
    /// The account lookup doubles as the credential check; its id is stored in the public data.
    async fn validate_credentials(&self) -> Result<ValidatedCredentials, Report<ConnectorError>> {
        use stripe_client::accounts::AccountsApi;
        let secret = self.secret_key();
        let publishable = self.publishable_key();
        let account = Self::client()
            .get_account(&secret)
            .await
            .map_err(|e| match &e {
                StripeError::Stripe(req) if matches!(req.http_status, 400 | 401 | 403) => {
                    let report = Report::new(ConnectorError::Configuration(
                        "Invalid API keys".to_string(),
                    ))
                    .attach(e.to_string());
                    if req.http_status == 400 {
                        report
                    } else {
                        report.attach_opaque(CredentialsRejected)
                    }
                }
                _ => Report::new(ConnectorError::Transport(format!(
                    "Couldn't reach Stripe to verify the keys: {e}"
                ))),
            })?;
        // Account details only: a key that can't read payment-method settings (a restricted
        // key without that scope) still works, and falls back to the account capabilities.
        let configuration = {
            use stripe_client::payment_method_configurations::PaymentMethodConfigurationsApi;
            Self::client()
                .default_payment_method_configuration(&secret)
                .await
                .inspect_err(|e| log::warn!("Stripe payment method settings unavailable: {e}"))
                .ok()
                .flatten()
        };
        let secret = secret.expose_secret();
        Ok(ValidatedCredentials {
            account: ProviderAccount {
                account_id: Some(account.id.clone()),
                display_name: account.display_name(),
                livemode: secret.starts_with("sk_live_") || secret.starts_with("rk_live_"),
                key_hint: Some(mask_secret(secret)),
                methods: stripe_methods(&account.capabilities, configuration.as_ref()),
                checked_at: common_domain::clock::now().naive_utc(),
            },
            public: PaymentConnectorPublicData::Stripe(StripePublicData {
                api_publishable_key: publishable.expose_secret().to_string(),
                account_id: Some(account.id),
                webhook_events_fingerprint: None,
            }),
        })
    }
}

/// Per rail: the Dashboard's payment-method settings decide when readable (direct accounts report
/// no capabilities); otherwise the account capabilities (Connect accounts). A rail neither
/// reports stays enabled, since blocking it could break a working setup.
fn stripe_methods(
    capabilities: &HashMap<String, String>,
    configuration: Option<&PaymentMethodConfiguration>,
) -> Vec<PaymentMethodAvailability> {
    STRIPE_CAPABILITIES
        .supported_payment_methods
        .iter()
        .map(|method| {
            let (method_type, capability) = match method {
                PaymentMethodTypeEnum::Card => ("card", "card_payments"),
                PaymentMethodTypeEnum::DirectDebitSepa => ("sepa_debit", "sepa_debit_payments"),
                PaymentMethodTypeEnum::DirectDebitAch => {
                    ("us_bank_account", "us_bank_account_ach_payments")
                }
                PaymentMethodTypeEnum::DirectDebitBacs => ("bacs_debit", "bacs_debit_payments"),
                PaymentMethodTypeEnum::Transfer | PaymentMethodTypeEnum::Other => ("", ""),
            };
            let setting = configuration.and_then(|c| c.method(method_type));
            let (state, reason) = match (setting, capabilities.get(capability).map(String::as_str))
            {
                (_, Some("pending")) => (
                    MethodState::Pending,
                    Some("Stripe is still reviewing this payment method".to_string()),
                ),
                (Some(s), _) if !s.available => (
                    MethodState::Unavailable,
                    Some("Not available on this Stripe account".to_string()),
                ),
                (Some(s), _) if !s.is_on() => (
                    MethodState::Unavailable,
                    Some("Turned off in Stripe's payment method settings".to_string()),
                ),
                (Some(_), _) => (MethodState::Enabled, None),
                (None, Some("inactive")) => (
                    MethodState::Unavailable,
                    Some("Not active on this Stripe account".to_string()),
                ),
                (None, _) => (MethodState::Enabled, None),
            };
            PaymentMethodAvailability {
                method: method.clone(),
                state,
                reason,
            }
        })
        .collect()
}

#[async_trait]
impl CustomerOps for StripeConnector {
    async fn create_customer(
        &self,
        customer: &Customer,
        request: CreateCustomerRequest,
    ) -> Result<ExternalCustomerRef, Report<ConnectorError>> {
        let secret_key = self.secret_key();

        let mut metadata = HashMap::from([
            ("meteroid.id".to_string(), customer.id.as_base62()),
            (
                "meteroid.tenant_id".to_string(),
                customer.tenant_id.as_base62(),
            ),
        ]);
        if let Some(alias) = &customer.alias {
            metadata.insert("meteroid.alias".to_string(), alias.clone());
        }

        let res = Self::client()
            .create_customer(
                CreateCustomer {
                    name: Some(customer.display_name()),
                    address: customer.billing_address.as_ref().map(map_address),
                    email: customer.billing_email.clone(),
                    source: None,
                    shipping: customer
                        .shipping_address
                        .as_ref()
                        .and_then(|a| a.address.as_ref())
                        .map(|a| CustomerShipping {
                            address: map_address(a),
                            name: customer.display_name(),
                            phone: customer.phone.clone(),
                        }),
                    metadata: Some(metadata),
                    phone: customer.phone.clone(),
                    description: None,
                    preferred_locales: None,
                    validate: None,
                },
                &secret_key,
                request.idempotency_key.as_str().to_string(),
            )
            .await
            .map_err(map_stripe_error)?;

        Ok(ExternalCustomerRef {
            external_id: res.id,
            provider_request_id: None,
        })
    }
}

#[async_trait]
impl MandateOps for StripeConnector {
    async fn initiate_mandate_setup(
        &self,
        connection: &CustomerConnection,
        request: MandateSetupRequest<'_>,
    ) -> Result<MandateSetupInstruction, Report<ConnectorError>> {
        let secret_key = self.secret_key();
        let publishable_key = self.publishable_key();

        let stripe_payment_methods: Vec<StripeSetupPmType> = request
            .payment_methods
            .iter()
            .filter_map(to_stripe_setup_pm_type)
            .collect();

        let metadata = HashMap::from([
            (
                "meteroid.tenant_id".to_string(),
                self.connector.tenant_id.as_base62(),
            ),
            (
                "meteroid.customer_id".to_string(),
                connection.customer_id.as_base62(),
            ),
            (
                "meteroid.connection_id".to_string(),
                connection.id.as_base62(),
            ),
            (
                SetupPurpose::METADATA_KEY.to_string(),
                request.purpose.as_str().to_string(),
            ),
        ]);

        let setup_intent = Self::client()
            .create_setup_intent(
                CreateSetupIntent {
                    customer: Some(connection.external_customer_id.clone()),
                    payment_method_types: Some(stripe_payment_methods),
                    usage: Some(CreateSetupIntentUsage::OffSession),
                    setup_mandate_details: None,
                    metadata,
                },
                &secret_key,
                request.idempotency_key.as_str().to_string(),
            )
            .await
            .map_err(map_stripe_error)?;

        Ok(MandateSetupInstruction::EmbeddedClientSecret {
            intent_id: setup_intent.id,
            client_secret: setup_intent.client_secret,
            publishable_key,
        })
    }

    async fn fetch_payment_method(
        &self,
        external_payment_method_id: &str,
        external_customer_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        let secret_key = self.secret_key();

        let method = Self::client()
            .get_payment_method(
                external_payment_method_id,
                external_customer_id,
                &secret_key,
            )
            .await
            .map_err(map_stripe_error)?;

        Ok(snapshot_from_payment_method(method))
    }

    async fn complete_mandate_setup(
        &self,
        _intent_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        // Stripe SetupIntents finalize client-side and confirm via the
        // `setup_intent.succeeded` webhook; no server-side complete step.
        Err(Report::new(ConnectorError::Unsupported {
            provider: self.provider().into(),
            capability: "mandate.complete (Stripe finalizes client-side via webhook)",
        }))
    }
}

#[async_trait]
impl PaymentOps for StripeConnector {
    async fn charge_off_session(
        &self,
        request: ChargeRequest<'_>,
    ) -> Result<ChargeOutcome, Report<ConnectorError>> {
        let secret_key = self.secret_key();
        // Non-secret; surfaced in next_action so the portal can init Stripe.js.
        let publishable_key = self.public.api_publishable_key.clone();

        let metadata = HashMap::from([
            (
                "meteroid.tenant_id".to_string(),
                self.connector.tenant_id.as_base62(),
            ),
            (
                "meteroid.transaction_id".to_string(),
                request.transaction_id.as_base62(),
            ),
        ]);

        let pm_type = to_stripe_setup_pm_type(&request.payment_method_type);

        let result = Self::client()
            .create_payment_intent(
                PaymentIntentRequest {
                    amount: request.amount_minor,
                    currency: request.currency.to_string(),
                    customer: Some(request.customer_external_id.to_string()),
                    setup_mandate_details: None,
                    payment_method: request.payment_method_external_id.to_string(),
                    confirm: true,
                    metadata,
                    // Off-session marks this as merchant-initiated; on-session
                    // lets Stripe return a completable `requires_action` (3DS).
                    off_session: Some(!request.on_session),
                    return_url: None,
                    capture_method: Default::default(),
                    payment_method_types: pm_type.into_iter().collect(),
                },
                &secret_key,
                request.idempotency_key.as_str().to_string(),
            )
            .await;

        match result {
            Ok(intent) => Ok(intent_to_outcome(intent, &publishable_key)),
            Err(StripeError::Stripe(req_err)) if req_err.http_status == 402 => {
                Ok(ChargeOutcome::Failed(card_error_to_failure(req_err)))
            }
            Err(e) => Err(map_stripe_error(e)),
        }
    }

    async fn fetch_pending_action(
        &self,
        external_transaction_id: &str,
    ) -> Result<Option<RequiresActionInstruction>, Report<ConnectorError>> {
        let secret_key = self.secret_key();
        let publishable_key = self.publishable_key();

        let intent = Self::client()
            .get_payment_intent(external_transaction_id, &secret_key)
            .await
            .map_err(map_stripe_error)?;

        // Any other status means the challenge was answered or lapsed meanwhile.
        if intent.status != StripePaymentStatus::RequiresCustomerAction {
            return Ok(None);
        }
        Ok(Some(requires_action_instruction(
            intent.id,
            intent.next_action,
            intent.client_secret,
            publishable_key.expose_secret(),
        )))
    }
}

#[async_trait]
impl RefundOps for StripeConnector {
    async fn refund(
        &self,
        request: RefundRequest<'_>,
    ) -> Result<RefundOutcome, Report<ConnectorError>> {
        let secret_key = self.secret_key();

        let mut metadata: HashMap<String, String> = request.metadata.into_iter().collect();
        // The tenant tag is what lets the dispatcher drop a refund event delivered
        // to another tenant's endpoint; never override what the caller stamped.
        metadata
            .entry("meteroid.tenant_id".to_string())
            .or_insert_with(|| self.connector.tenant_id.as_base62());
        // Stripe's `reason` enum has no free-text member, so a merchant-typed
        // reason rides along as metadata instead of being dropped.
        if let Some(RefundReason::Other(text)) = &request.reason {
            metadata.insert("meteroid.refund_reason".to_string(), text.clone());
        }

        let refund = Self::client()
            .create_refund(
                CreateRefund {
                    payment_intent: request.external_transaction_id.to_string(),
                    amount: Some(request.amount_minor),
                    reason: request.reason.as_ref().and_then(to_stripe_refund_reason),
                    metadata,
                },
                &secret_key,
                request.idempotency_key.as_str().to_string(),
            )
            .await
            .map_err(map_stripe_error)?;

        Ok(refund_to_outcome(refund))
    }

    async fn fetch_refund(
        &self,
        external_refund_id: &str,
        _external_transaction_id: Option<&str>,
    ) -> Result<RefundSnapshot, Report<ConnectorError>> {
        let secret_key = self.secret_key();

        let refund = Self::client()
            .get_refund(external_refund_id, &secret_key)
            .await
            .map_err(map_stripe_error)?;

        Ok(refund_snapshot(refund))
    }
}

#[async_trait]
impl ReconcileOps for StripeConnector {
    async fn fetch_transaction_status(
        &self,
        external_transaction_id: &str,
    ) -> Result<RemoteTransactionStatus, Report<ConnectorError>> {
        let secret_key = self.secret_key();

        let result = Self::client()
            .get_payment_intent(external_transaction_id, &secret_key)
            .await;

        match result {
            Ok(intent) => Ok(remote_status_from_intent(intent)),
            // 404 = Stripe has no record of the charge; reconciliation worker
            // treats Unknown as safe-to-cancel locally.
            Err(StripeError::Stripe(req_err)) if req_err.http_status == 404 => {
                Ok(RemoteTransactionStatus::Unknown)
            }
            Err(e) => Err(map_stripe_error(e)),
        }
    }
}

/// Map a (possibly already-settled) Stripe payment intent to the
/// reconciliation status enum the worker uses.
///
/// PaymentIntents have no `failed` status: a failed attempt lands the intent
/// back in `requires_payment_method`/`requires_confirmation` with
/// `last_payment_error` populated, which is the only way reconciliation can
/// ever observe a failure. A pristine intent in those statuses (no attempt
/// yet — e.g. a portal checkout still awaiting the customer) has no
/// `last_payment_error` and must stay `Pending`.
fn remote_status_from_intent(intent: StripePaymentIntent) -> RemoteTransactionStatus {
    match intent.status {
        StripePaymentStatus::Succeeded => RemoteTransactionStatus::Succeeded {
            amount_received_minor: intent.amount_received.unwrap_or(intent.amount),
            currency: intent.currency.clone(),
            // `StripePaymentIntent` doesn't carry Stripe's `created` timestamp
            // (stripe-client crate); wall-clock is a stand-in until it's added.
            processed_at: common_domain::clock::now().naive_utc(),
        },
        StripePaymentStatus::RequiresPaymentMethod | StripePaymentStatus::RequiresConfirmation
            if intent.last_payment_error.is_some() =>
        {
            RemoteTransactionStatus::Failed {
                code: stripe_error_code(&intent.last_payment_error),
                message: flatten_payment_error(&intent.last_payment_error)
                    .unwrap_or_else(|| "payment failed".to_string()),
                decline_kind: decline_kind_from_error(&intent.last_payment_error),
            }
        }
        StripePaymentStatus::Pending
        | StripePaymentStatus::Processing
        | StripePaymentStatus::RequiresCapture => RemoteTransactionStatus::Processing,
        StripePaymentStatus::RequiresCustomerAction
        | StripePaymentStatus::RequiresPaymentMethod
        | StripePaymentStatus::RequiresConfirmation => RemoteTransactionStatus::Pending,
        StripePaymentStatus::Canceled => RemoteTransactionStatus::Cancelled,
        // Source-only statuses, never returned for a PaymentIntent GET; kept
        // as a defensive Failed mapping rather than a panic/Unknown.
        StripePaymentStatus::Failed
        | StripePaymentStatus::Chargeable
        | StripePaymentStatus::Consumed => RemoteTransactionStatus::Failed {
            code: stripe_error_code(&intent.last_payment_error),
            message: flatten_payment_error(&intent.last_payment_error)
                .unwrap_or_else(|| "payment failed".to_string()),
            decline_kind: decline_kind_from_error(&intent.last_payment_error),
        },
    }
}

#[async_trait]
impl WebhookOps for StripeConnector {
    async fn register_webhook(
        &self,
        url: &str,
        subscriptions: &[NormalizedEventSubscription],
    ) -> Result<RegisteredWebhook, Report<ConnectorError>> {
        let secret_key = self.secret_key();
        let enabled_events = subscriptions_to_stripe_events(subscriptions);

        let endpoint = Self::client()
            .create_webhook_endpoint(
                CreateWebhookEndpointRequest {
                    url: url.to_string(),
                    enabled_events,
                    description: Some(format!("Meteroid (tenant {})", self.connector.tenant_id)),
                },
                &secret_key,
                // Idempotency key must be stable across retries: keyed on
                // (tenant_id, alias, url), never `connector.id` (freshly
                // generated per call) — an unstable key orphans an endpoint per retry.
                format!(
                    "webhook_register:{}:{}:{}",
                    self.connector.tenant_id, self.connector.alias, url
                ),
            )
            .await
            .map_err(map_stripe_error)?;

        let secret = endpoint.secret.ok_or_else(|| {
            Report::new(ConnectorError::WebhookRegistration(
                "Stripe returned an endpoint without a secret (unexpected)".into(),
            ))
        })?;

        Ok(RegisteredWebhook {
            endpoint_id: endpoint.id,
            secret: SecretString::from(secret),
        })
    }

    async fn unregister_webhook(&self, endpoint_id: &str) -> Result<(), Report<ConnectorError>> {
        let secret_key = self.secret_key();

        Self::client()
            .delete_webhook_endpoint(endpoint_id, &secret_key)
            .await
            .map(|_| ())
            .map_err(map_stripe_error)
    }

    async fn sync_webhook_events(
        &self,
        endpoint_id: &str,
        subscriptions: &[NormalizedEventSubscription],
    ) -> Result<(), Report<ConnectorError>> {
        let secret_key = self.secret_key();
        let enabled_events = subscriptions_to_stripe_events(subscriptions);
        // Stripe refuses a reused key with a different body, so the key must change with the
        // event list: a release that adds events re-syncs instead of replaying the old set.
        let idempotency_key = format!(
            "webhook_sync:{}:{}:{}",
            self.connector.tenant_id,
            endpoint_id,
            self.webhook_subscription_fingerprint(subscriptions)
        );

        Self::client()
            .update_webhook_endpoint(
                endpoint_id,
                UpdateWebhookEndpointRequest { enabled_events },
                &secret_key,
                idempotency_key,
            )
            .await
            .map(|_| ())
            .map_err(map_stripe_error)
    }

    fn webhook_subscription_fingerprint(
        &self,
        subscriptions: &[NormalizedEventSubscription],
    ) -> String {
        subscription_fingerprint(&subscriptions_to_stripe_events(subscriptions))
    }

    fn verify_signature(
        &self,
        payload: &[u8],
        headers: &HeaderMap,
        secret: &SecretString,
    ) -> Result<(), Report<ConnectorError>> {
        let sig = headers
            .get("Stripe-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| Report::new(ConnectorError::SignatureMissing))?;

        let body = std::str::from_utf8(payload).map_err(|e| {
            Report::new(ConnectorError::PayloadDecode(format!(
                "payload is not valid utf-8: {e}"
            )))
        })?;

        StripeWebhook::validate_signature(body, sig, secret.expose_secret())
            .map_err(|_| Report::new(ConnectorError::SignatureVerification))
    }

    fn parse_event(
        &self,
        payload: &[u8],
        _headers: &HeaderMap,
    ) -> Result<Option<NormalizedWebhookEvent>, Report<ConnectorError>> {
        let body = std::str::from_utf8(payload).map_err(|e| {
            Report::new(ConnectorError::PayloadDecode(format!(
                "payload is not valid utf-8: {e}"
            )))
        })?;

        let parsed = StripeWebhook::parse_event(body).map_err(|e| {
            Report::new(ConnectorError::PayloadDecode(format!(
                "failed to decode stripe event: {e}"
            )))
        })?;

        Ok(normalize_event(parsed))
    }
}

// ── helpers ────────────────────────────────────────────────────────────────

fn map_address(a: &Address) -> OptionalFieldsAddress {
    OptionalFieldsAddress {
        city: a.city.clone(),
        country: a.country.clone(),
        line1: a.line1.clone(),
        line2: a.line2.clone(),
        state: a.state.clone(),
        postal_code: a.zip_code.clone(),
    }
}

fn to_stripe_setup_pm_type(method: &PaymentMethodTypeEnum) -> Option<StripeSetupPmType> {
    match method {
        PaymentMethodTypeEnum::Card => Some(StripeSetupPmType::Card),
        PaymentMethodTypeEnum::DirectDebitSepa => Some(StripeSetupPmType::Sepa),
        PaymentMethodTypeEnum::DirectDebitAch => Some(StripeSetupPmType::Ach),
        PaymentMethodTypeEnum::DirectDebitBacs => Some(StripeSetupPmType::Bacs),
        PaymentMethodTypeEnum::Other | PaymentMethodTypeEnum::Transfer => None,
    }
}

fn from_stripe_pm_type(t: &StripePmType) -> PaymentMethodTypeEnum {
    match t {
        StripePmType::Card => PaymentMethodTypeEnum::Card,
        StripePmType::SepaDebit => PaymentMethodTypeEnum::DirectDebitSepa,
        StripePmType::UsBankAccount => PaymentMethodTypeEnum::DirectDebitAch,
        StripePmType::BacsDebit => PaymentMethodTypeEnum::DirectDebitBacs,
        StripePmType::Other => PaymentMethodTypeEnum::Other,
    }
}

fn snapshot_from_payment_method(method: PaymentMethod) -> PaymentMethodSnapshot {
    let payment_method_type = from_stripe_pm_type(&method._type);

    let account_number_hint = match method._type {
        StripePmType::Card => None,
        StripePmType::BacsDebit => method.bacs_debit.as_ref().and_then(|a| a.last4.clone()),
        StripePmType::SepaDebit => method.sepa_debit.as_ref().and_then(|a| a.last4.clone()),
        StripePmType::UsBankAccount => method
            .us_bank_account
            .as_ref()
            .and_then(|a| a.last4.clone()),
        StripePmType::Other => None,
    };

    let (card_brand, card_last4, card_exp_month, card_exp_year) =
        match (&method._type, &method.card) {
            (StripePmType::Card, Some(card)) => (
                Some(card.brand.clone()),
                card.last4.clone(),
                Some(card.exp_month),
                Some(card.exp_year),
            ),
            _ => (None, None, None, None),
        };

    let fingerprint = match method._type {
        StripePmType::Card => method.card.as_ref().and_then(|c| c.fingerprint.clone()),
        StripePmType::SepaDebit => method
            .sepa_debit
            .as_ref()
            .and_then(|a| a.fingerprint.clone()),
        StripePmType::UsBankAccount => method
            .us_bank_account
            .as_ref()
            .and_then(|a| a.fingerprint.clone()),
        StripePmType::BacsDebit => method
            .bacs_debit
            .as_ref()
            .and_then(|a| a.fingerprint.clone()),
        StripePmType::Other => None,
    };

    PaymentMethodSnapshot {
        external_payment_method_id: method.id,
        payment_method_type,
        account_number_hint,
        card_brand,
        card_last4,
        card_exp_month,
        card_exp_year,
        fingerprint,
        // Stripe events carry our metadata directly.
        meteroid_connection_id: None,
        meteroid_customer_id: None,
        // Stripe invoice payments charge inline (no post-mandate webhook charge).
        meteroid_invoice_id: None,
        // Hosted combined mandate+payment checkout is GoCardless-only.
        meteroid_checkout_session_id: None,
        meteroid_transaction_id: None,
        payment_request_payment: None,
    }
}

/// Map a fresh `StripePaymentIntent` (returned from a create-and-confirm call)
/// to a normalized [`ChargeOutcome`]. Status transitions in Stripe:
///
/// - `succeeded` → `Succeeded`
/// - `processing` / `pending` → `Pending` (async settlement: ACH, BACS)
/// - `requires_action` → `RequiresAction` (3DS / SCA — needs a follow-up flow)
/// - `requires_capture` → `Pending`: the intent is authorized, not failed;
///   retrying the charge while the auth stands risks a second authorization.
/// - `requires_payment_method` / `requires_confirmation` → `Failed`,
///   `retryable=false`; in practice the customer has to re-authorize from the
///   portal.
/// - `canceled` → `Cancelled`; `failed` → `Failed` (both terminal).
fn intent_to_outcome(intent: StripePaymentIntent, publishable_key: &str) -> ChargeOutcome {
    match intent.status {
        StripePaymentStatus::Succeeded => ChargeOutcome::Succeeded(ChargeReceipt {
            external_id: intent.id,
            amount_received_minor: intent.amount_received.unwrap_or(intent.amount),
            // Same caveat as `remote_status_from_intent`: no provider timestamp
            // available on `StripePaymentIntent` yet, so this uses wall-clock.
            processed_at: common_domain::clock::now().naive_utc(),
            provider_request_id: None,
        }),
        StripePaymentStatus::Pending | StripePaymentStatus::Processing => {
            ChargeOutcome::Pending(ChargeAcknowledged {
                external_id: intent.id,
                provider_request_id: None,
            })
        }
        // Authorized but not yet captured: not a failure. Treat like an async
        // settlement in progress rather than `Failed`, since the funds are held.
        StripePaymentStatus::RequiresCapture => ChargeOutcome::Pending(ChargeAcknowledged {
            external_id: intent.id,
            provider_request_id: None,
        }),
        // 3DS/SCA: `next_action` says what the client must do. See requires_action_instruction.
        StripePaymentStatus::RequiresCustomerAction => {
            ChargeOutcome::RequiresAction(requires_action_instruction(
                intent.id,
                intent.next_action,
                intent.client_secret,
                publishable_key,
            ))
        }
        StripePaymentStatus::RequiresPaymentMethod
        | StripePaymentStatus::RequiresConfirmation
        | StripePaymentStatus::Chargeable
        | StripePaymentStatus::Consumed => ChargeOutcome::Failed(ChargeFailure {
            external_id: Some(intent.id.clone()),
            code: Some(format!("{:?}", intent.status).to_lowercase()),
            message: flatten_payment_error(&intent.last_payment_error)
                .unwrap_or_else(|| format!("Unexpected status: {:?}", intent.status)),
            // Not retryable: `idempotency_key` is derived from `transaction_id`,
            // so a blind retry would just replay this same cached Stripe
            // response rather than making progress — no automatic re-charge.
            retryable: false,
            decline_kind: DeclineKind::ProcessingError,
            provider_request_id: None,
        }),
        StripePaymentStatus::Failed => ChargeOutcome::Failed(ChargeFailure {
            external_id: Some(intent.id),
            code: stripe_error_code(&intent.last_payment_error),
            message: flatten_payment_error(&intent.last_payment_error)
                .unwrap_or_else(|| "Payment failed".to_string()),
            retryable: false,
            decline_kind: decline_kind_from_error(&intent.last_payment_error),
            provider_request_id: None,
        }),
        StripePaymentStatus::Canceled => ChargeOutcome::Cancelled(ChargeCancelled {
            external_id: Some(intent.id),
            message: "Payment canceled".to_string(),
            provider_request_id: None,
        }),
    }
}

/// Prefer the hosted-redirect URL (works in any browser); fall back to the
/// ClientSecret form for SDK-driven flows (Stripe.js, mobile SDK).
fn requires_action_instruction(
    intent_id: String,
    next_action: Option<StripeNextAction>,
    client_secret: Option<String>,
    publishable_key: &str,
) -> RequiresActionInstruction {
    // A bare redirect (rare for cards) can be opened directly. Everything else
    // is SDK-driven: the portal calls Stripe.js `handleNextAction` with the
    // PaymentIntent client_secret (returned on create), so carry it through.
    let redirect_url = next_action
        .as_ref()
        .and_then(|a| a.redirect_to_url.as_ref())
        .and_then(|r| r.url.clone());

    match redirect_url {
        Some(url) => RequiresActionInstruction::HostedUrl {
            external_id: intent_id,
            url,
            expires_at: None,
        },
        None => RequiresActionInstruction::ClientSecret {
            external_id: intent_id,
            client_secret: client_secret.unwrap_or_default(),
            publishable_key: SecretString::from(publishable_key.to_string()),
        },
    }
}

/// `last_payment_error` is a nested `{ code, message, decline_code, ... }`;
/// surface the human-readable message.
fn flatten_payment_error(err: &Option<StripePaymentError>) -> Option<String> {
    err.as_ref().and_then(|e| e.message.clone())
}

fn stripe_error_code(err: &Option<StripePaymentError>) -> Option<String> {
    err.as_ref().and_then(|e| e.code.clone())
}

/// Covers the high-frequency decline codes (rest fall back to `Other`); the
/// retry-policy layer uses the result to decide whether to retry the same card.
fn decline_kind_from_error(err: &Option<StripePaymentError>) -> DeclineKind {
    let err = err.as_ref();
    decline_kind_from_code(
        err.and_then(|e| e.decline_code.as_deref().or(e.code.as_deref())),
        err.and_then(|e| e.advice_code.as_deref()),
    )
}

fn decline_kind_from_code(code: Option<&str>, advice_code: Option<&str>) -> DeclineKind {
    if advice_code == Some("do_not_try_again") {
        return DeclineKind::DoNotRetry;
    }
    let Some(decline_code) = code else {
        return DeclineKind::Other;
    };
    match decline_code {
        // Card-network "do not reattempt" responses: re-presenting them draws scheme fees.
        "pickup_card"
        | "restricted_card"
        | "invalid_account"
        | "transaction_not_allowed"
        | "stop_payment_order"
        | "revocation_of_authorization"
        | "revocation_of_all_authorizations"
        | "security_violation"
        | "do_not_try_again" => DeclineKind::DoNotRetry,
        "new_account_information_available" => DeclineKind::CardExpired,
        "insufficient_funds" => DeclineKind::InsufficientFunds,
        // Off-session charges submit no CVC, so a CVC failure says nothing about the card.
        "do_not_honor" | "generic_decline" | "incorrect_cvc" => DeclineKind::DoNotHonor,
        "expired_card" | "incorrect_number" => DeclineKind::CardExpired,
        "authentication_required" => DeclineKind::AuthenticationRequired,
        "fraudulent" | "stolen_card" | "lost_card" => DeclineKind::Fraud,
        "processing_error" | "issuer_not_available" | "try_again_later" => {
            DeclineKind::ProcessingError
        }
        _ => DeclineKind::Other,
    }
}

/// Only Stripe's three enum members survive; `Other` is free text Stripe rejects,
/// so `refund()` sends it as `meteroid.refund_reason` metadata instead.
fn to_stripe_refund_reason(reason: &RefundReason) -> Option<StripeRefundReason> {
    match reason {
        RefundReason::Duplicate => Some(StripeRefundReason::Duplicate),
        RefundReason::Fraudulent => Some(StripeRefundReason::Fraudulent),
        RefundReason::RequestedByCustomer => Some(StripeRefundReason::RequestedByCustomer),
        RefundReason::Other(_) => None,
    }
}

/// An unknown or absent status stays `Pending`: the reconcile sweep re-reads the
/// refund rather than the row being settled or failed on a guess.
fn remote_refund_status(refund: &StripeRefund) -> RemoteRefundStatus {
    match refund.status.as_deref() {
        Some(refund_status::SUCCEEDED) => RemoteRefundStatus::Succeeded,
        Some(refund_status::CANCELED) => RemoteRefundStatus::Canceled,
        Some(refund_status::FAILED) => RemoteRefundStatus::Failed {
            code: refund.failure_reason.clone(),
            message: refund
                .failure_reason
                .clone()
                .unwrap_or_else(|| "Refund failed".to_string()),
        },
        _ => RemoteRefundStatus::Pending,
    }
}

fn refund_processed_at(refund: &StripeRefund) -> chrono::NaiveDateTime {
    refund
        .created
        .and_then(|ts| DateTime::from_timestamp(ts, 0))
        .unwrap_or_else(common_domain::clock::now)
        .naive_utc()
}

/// `Canceled` is a refund Stripe gave up on — the funds never left, so the row is
/// `Failed` like a decline, just without provider error text.
fn refund_to_outcome(refund: StripeRefund) -> RefundOutcome {
    match remote_refund_status(&refund) {
        RemoteRefundStatus::Succeeded => RefundOutcome::Succeeded(RefundReceipt {
            amount_refunded_minor: refund.amount,
            processed_at: refund_processed_at(&refund),
            external_refund_id: refund.id,
            provider_request_id: None,
        }),
        RemoteRefundStatus::Pending => RefundOutcome::Pending(RefundAcknowledged {
            external_refund_id: refund.id,
            provider_request_id: None,
        }),
        RemoteRefundStatus::Failed { code, message } => RefundOutcome::Failed(RefundFailure {
            code,
            message,
            provider_request_id: None,
        }),
        RemoteRefundStatus::Canceled => RefundOutcome::Failed(RefundFailure {
            code: Some(refund_status::CANCELED.to_string()),
            message: "Refund canceled by Stripe".to_string(),
            provider_request_id: None,
        }),
    }
}

fn refund_snapshot(refund: StripeRefund) -> RefundSnapshot {
    RefundSnapshot {
        external_transaction_id: refund
            .payment_intent
            .clone()
            .or_else(|| refund.charge.clone())
            .unwrap_or_default(),
        amount_minor: refund.amount,
        status: remote_refund_status(&refund),
        // Stripe reports the cumulative refunded total on the charge, never on a refund.
        cumulative_refunded_minor: None,
        currency: refund.currency.clone().unwrap_or_default(),
        external_refund_id: refund.id,
    }
}

fn observed_refunds_on_wire(refunds: &Option<StripeRefundList>) -> Vec<ObservedRefundOnWire> {
    refunds
        .as_ref()
        .map(|list| {
            list.data
                .iter()
                .map(|r| ObservedRefundOnWire {
                    external_refund_id: r.id.clone(),
                    amount_minor: Some(r.amount),
                    status: match r.status.as_deref() {
                        Some(refund_status::SUCCEEDED) => ObservedRefundStatus::Succeeded,
                        Some(refund_status::FAILED) | Some(refund_status::CANCELED) => {
                            ObservedRefundStatus::Failed
                        }
                        _ => ObservedRefundStatus::Pending,
                    },
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A confirm-time decline (HTTP 402) is a definitive outcome, not an error: Stripe has
/// recorded the failed attempt under this key, so it is reported like a `failed` intent.
fn card_error_to_failure(req_err: stripe_client::error::RequestError) -> ChargeFailure {
    let code = req_err.decline_code.clone().or(req_err.code.clone());
    ChargeFailure {
        external_id: req_err.payment_intent.map(|pi| pi.id),
        decline_kind: decline_kind_from_code(code.as_deref(), req_err.advice_code.as_deref()),
        code,
        message: req_err
            .message
            .unwrap_or_else(|| "Payment declined".to_string()),
        retryable: false,
        provider_request_id: None,
    }
}

/// Network failures, Stripe 5xx, rate limits and idempotency conflicts map to `Transport` (the
/// outcome is unknown, so never treat it as a decline); other 4xx map to
/// `Charge`/`Configuration` (not retryable).
fn map_stripe_error(e: StripeError) -> Report<ConnectorError> {
    match e {
        StripeError::Timeout => Report::new(ConnectorError::Transport("stripe timeout".into())),
        StripeError::ClientError(msg) => Report::new(ConnectorError::Transport(msg)),
        StripeError::Stripe(req_err) if req_err.http_status >= 500 => Report::new(
            ConnectorError::Transport(format!("stripe 5xx: {}", req_err)),
        ),
        StripeError::Stripe(req_err)
            if matches!(req_err.http_status, 409 | 429)
                || req_err.error_type == "idempotency_error" =>
        {
            Report::new(ConnectorError::Transport(format!(
                "stripe idempotency conflict: {}",
                req_err
            )))
        }
        StripeError::Stripe(req_err) => {
            let rejected_key = matches!(req_err.http_status, 401 | 403);
            let report = Report::new(ConnectorError::Charge(format!(
                "stripe rejected: {} (code={:?})",
                req_err.message.unwrap_or_default(),
                req_err.code
            )));
            if rejected_key {
                report.attach_opaque(CredentialsRejected)
            } else {
                report
            }
        }
        StripeError::QueryStringSerialize(_)
        | StripeError::JSONSerialize(_)
        | StripeError::UnsupportedVersion => {
            Report::new(ConnectorError::Configuration(format!("stripe sdk: {e}")))
        }
    }
}

/// Returns `None` for events recognized at the parse layer but not surfaced.
fn normalize_event(parsed: Event) -> Option<NormalizedWebhookEvent> {
    // Prefer Stripe's `created` ts; wall-clock fallback would break
    // dispute-window math and replays, so only use it if the field is missing.
    let occurred_at = parsed
        .created
        .and_then(|ts| DateTime::from_timestamp(ts, 0))
        .unwrap_or_else(common_domain::clock::now);
    let event_type = parsed.event_type.clone();
    let id = parsed.id.clone();

    let owner_tenant_id = object_owner_tenant_id(&parsed.data.object);
    let kind = normalize_kind(event_type.as_str(), parsed.data.object)?;

    Some(NormalizedWebhookEvent {
        provider_event_id: id,
        provider_event_type: event_type,
        occurred_at,
        kind,
        owner_tenant_id,
    })
}

/// Our `meteroid.tenant_id` off a Stripe object's metadata, when present.
fn object_owner_tenant_id(object: &EventObject) -> Option<String> {
    let tenant =
        |m: &std::collections::HashMap<String, String>| m.get("meteroid.tenant_id").cloned();
    match object {
        EventObject::PaymentIntent(pi) => tenant(&pi.metadata),
        EventObject::SetupIntent(si) => tenant(&si.metadata),
        EventObject::Refund(r) => tenant(&r.metadata),
        // Others are narrowed to the fields we use and carry no metadata here.
        _ => None,
    }
}

fn normalize_kind(event_type: &str, object: EventObject) -> Option<NormalizedEventKind> {
    use stripe_client::webhook::StripeMandateStatus;

    Some(match (event_type, object) {
        // ── SetupIntent ──────────────────────────────────────────────
        (event_type::SETUP_INTENT_SUCCEEDED, EventObject::SetupIntent(si)) => {
            let payment_method = si.payment_method?;
            let external_customer_id = si.customer?;
            NormalizedEventKind::PaymentMethodAttached(PaymentMethodAttachedEvent {
                external_customer_id,
                external_payment_method_id: payment_method,
                // PM type unknown without an extra fetch; handler calls fetch_payment_method.
                payment_method_type: PaymentMethodTypeEnum::Other,
                meteroid_connection_id: si.metadata.get("meteroid.connection_id").cloned(),
                meteroid_customer_id: si.metadata.get("meteroid.customer_id").cloned(),
                purpose: si
                    .metadata
                    .get(SetupPurpose::METADATA_KEY)
                    .and_then(|purpose| SetupPurpose::parse(purpose)),
            })
        }
        (event_type::SETUP_INTENT_REQUIRES_ACTION, EventObject::SetupIntent(_si)) => {
            // A SetupIntent saves a card/mandate — it has no PaymentTransaction and
            // thus no `meteroid.transaction_id`. The 3DS/SCA challenge is driven
            // client-side by Stripe.js using the SetupIntent client_secret returned
            // at creation; the backend acts only on the terminal `setup_intent.succeeded`
            // (→ PaymentMethodAttached). Nothing to persist here — just ack it.
            NormalizedEventKind::Acknowledged {
                reason: "setup_intent.requires_action — 3DS handled client-side",
            }
        }
        (event_type::SETUP_INTENT_CANCELED, EventObject::SetupIntent(si)) => {
            NormalizedEventKind::PaymentMethodDetached(PaymentMethodDetachedEvent {
                external_payment_method_id: si.payment_method.unwrap_or_default(),
                reason: Some("setup_intent.canceled".into()),
            })
        }

        // ── PaymentIntent ────────────────────────────────────────────
        (event_type::PAYMENT_INTENT_SUCCEEDED, EventObject::PaymentIntent(pi)) => {
            NormalizedEventKind::PaymentSucceeded(PaymentSucceededEvent {
                external_transaction_id: pi.id,
                amount_received_minor: pi.amount_received.unwrap_or(pi.amount),
                currency: pi.currency,
                meteroid_transaction_id: pi.metadata.get("meteroid.transaction_id").cloned(),
            })
        }
        // A failed attempt on a prepared top-up intent may still be retried from its exposed
        // client secret, so the top-up workflow, not this event, closes its payment.
        (event_type::PAYMENT_INTENT_FAILED, EventObject::PaymentIntent(pi))
            if pi
                .metadata
                .get(wallet::PAYMENT_FLOW_METADATA_KEY)
                .map(String::as_str)
                == Some(wallet::WALLET_PAYMENT_FLOW) =>
        {
            NormalizedEventKind::Acknowledged {
                reason: "payment_intent.payment_failed on a prepared top-up payment",
            }
        }
        (event_type::PAYMENT_INTENT_FAILED, EventObject::PaymentIntent(pi)) => {
            NormalizedEventKind::PaymentFailed(PaymentFailedEvent {
                external_transaction_id: pi.id.clone(),
                code: stripe_error_code(&pi.last_payment_error),
                message: flatten_payment_error(&pi.last_payment_error)
                    .unwrap_or_else(|| "payment failed".to_string()),
                retryable: matches!(
                    decline_kind_from_error(&pi.last_payment_error),
                    DeclineKind::ProcessingError | DeclineKind::AuthenticationRequired
                ),
                decline_kind: decline_kind_from_error(&pi.last_payment_error),
                meteroid_transaction_id: pi.metadata.get("meteroid.transaction_id").cloned(),
            })
        }
        (event_type::PAYMENT_INTENT_REQUIRES_ACTION, EventObject::PaymentIntent(pi)) => {
            let action_url = pi
                .next_action
                .as_ref()
                .and_then(|n| n.redirect_to_url.as_ref())
                .and_then(|r| r.url.clone());
            NormalizedEventKind::PaymentRequiresAction(PaymentRequiresActionEvent {
                external_transaction_id: pi.id,
                action_url,
                client_secret: None,
                meteroid_transaction_id: pi.metadata.get("meteroid.transaction_id").cloned(),
            })
        }
        (event_type::PAYMENT_INTENT_PROCESSING, EventObject::PaymentIntent(pi)) => {
            NormalizedEventKind::PaymentPending(PaymentPendingEvent {
                external_transaction_id: pi.id,
                meteroid_transaction_id: pi.metadata.get("meteroid.transaction_id").cloned(),
                awaiting_customer: false,
            })
        }
        (event_type::PAYMENT_INTENT_PARTIALLY_FUNDED, EventObject::PaymentIntent(_pi)) => {
            // US ACH partial settlement: stay pending, the next event resolves it.
            NormalizedEventKind::Acknowledged {
                reason: "payment_intent.partially_funded — partial settlement",
            }
        }

        // ── Charge (refunds) ─────────────────────────────────────────
        (event_type::CHARGE_REFUNDED, EventObject::Charge(charge)) => {
            // Stripe pushes one event per refund; `refunds.data` surfaces the
            // most recent refund's id, used only to identify *which* refund
            // triggered this event.
            let parent_pi = charge.payment_intent.clone().unwrap_or_default();
            let refund_id = charge
                .refunds
                .as_ref()
                .and_then(|r| r.data.last())
                .map(|r| r.id.clone())
                .unwrap_or_default();
            let refunds = observed_refunds_on_wire(&charge.refunds);
            // `amount_refunded_minor` is always the charge's cumulative
            // refunded total (`charge.amount_refunded`), never a single
            // refund's delta; `refunds` carries the individual ids so the
            // handler can match what it already knows and record only the delta.
            NormalizedEventKind::PaymentRefunded(PaymentRefundedEvent {
                external_transaction_id: parent_pi,
                external_refund_id: refund_id,
                amount_refunded_minor: charge.amount_refunded,
                currency: charge.currency,
                refunds,
            })
        }

        // ── Refund lifecycle ─────────────────────────────────────────
        (event_type::REFUND_UPDATED, EventObject::Refund(refund))
        | (event_type::CHARGE_REFUND_UPDATED, EventObject::Refund(refund)) => {
            NormalizedEventKind::RefundStatusChanged(RefundStatusChangedEvent {
                external_transaction_id: refund
                    .payment_intent
                    .clone()
                    .or_else(|| refund.charge.clone()),
                status: remote_refund_status(&refund),
                amount_minor: Some(refund.amount),
                external_refund_id: refund.id,
            })
        }

        // ── Disputes ─────────────────────────────────────────────────
        (event_type::CHARGE_DISPUTE_CREATED, EventObject::Dispute(d)) => {
            NormalizedEventKind::DisputeOpened(dispute_event(d))
        }
        (event_type::CHARGE_DISPUTE_FUNDS_WITHDRAWN, EventObject::Dispute(d)) => {
            NormalizedEventKind::DisputeFundsWithdrawn(dispute_event(d))
        }
        (event_type::CHARGE_DISPUTE_FUNDS_REINSTATED, EventObject::Dispute(d)) => {
            NormalizedEventKind::DisputeFundsReinstated(dispute_event(d))
        }
        (event_type::CHARGE_DISPUTE_CLOSED, EventObject::Dispute(d)) => {
            // `dispute.closed` is terminal — `status` tells us who won.
            match d.status.as_str() {
                "won" => NormalizedEventKind::DisputeWon(dispute_event(d)),
                "lost" => NormalizedEventKind::DisputeLost(dispute_event(d)),
                _ => NormalizedEventKind::Acknowledged {
                    reason: "dispute.closed with non-terminal status",
                },
            }
        }

        // ── PaymentMethod lifecycle ──────────────────────────────────
        (event_type::PAYMENT_METHOD_UPDATED, EventObject::PaymentMethod(pm))
        | (event_type::PAYMENT_METHOD_AUTO_UPDATED, EventObject::PaymentMethod(pm)) => {
            NormalizedEventKind::PaymentMethodUpdated(payment_method_update(pm))
        }
        (event_type::PAYMENT_METHOD_DETACHED, EventObject::PaymentMethod(pm)) => {
            NormalizedEventKind::PaymentMethodDetached(PaymentMethodDetachedEvent {
                external_payment_method_id: pm.id,
                reason: Some("payment_method.detached".into()),
            })
        }

        // ── Mandate ──────────────────────────────────────────────────
        (event_type::MANDATE_UPDATED, EventObject::Mandate(m)) => match m.status {
            StripeMandateStatus::Inactive => {
                NormalizedEventKind::PaymentMethodDetached(PaymentMethodDetachedEvent {
                    external_payment_method_id: m.payment_method,
                    reason: Some(format!("mandate.{}", "inactive")),
                })
            }
            // Non-terminal mandate status; not acted on.
            _ => NormalizedEventKind::Acknowledged {
                reason: "mandate.updated — non-terminal status",
            },
        },

        _ => NormalizedEventKind::Acknowledged {
            reason: "unhandled stripe event type",
        },
    })
}

/// Must stay in sync with the [`normalize_kind`] dispatch table: anything we
/// parse must be registered here, or Stripe never sends it.
fn subscriptions_to_stripe_events(subs: &[NormalizedEventSubscription]) -> Vec<String> {
    use stripe_client::webhook::event_type as et;
    let mut events: Vec<&'static str> = Vec::new();
    for sub in subs {
        match sub {
            NormalizedEventSubscription::Payments => {
                events.extend([
                    et::PAYMENT_INTENT_SUCCEEDED,
                    et::PAYMENT_INTENT_FAILED,
                    et::PAYMENT_INTENT_REQUIRES_ACTION,
                    et::PAYMENT_INTENT_PROCESSING,
                    et::PAYMENT_INTENT_PARTIALLY_FUNDED,
                ]);
            }
            NormalizedEventSubscription::Mandates => {
                events.extend([
                    et::SETUP_INTENT_SUCCEEDED,
                    et::SETUP_INTENT_REQUIRES_ACTION,
                    et::SETUP_INTENT_CANCELED,
                    et::PAYMENT_METHOD_UPDATED,
                    et::PAYMENT_METHOD_AUTO_UPDATED,
                    et::PAYMENT_METHOD_DETACHED,
                    et::MANDATE_UPDATED,
                ]);
            }
            NormalizedEventSubscription::Refunds => {
                events.extend([
                    et::CHARGE_REFUNDED,
                    et::REFUND_UPDATED,
                    et::CHARGE_REFUND_UPDATED,
                ]);
            }
            NormalizedEventSubscription::Disputes => {
                events.extend([
                    et::CHARGE_DISPUTE_CREATED,
                    et::CHARGE_DISPUTE_CLOSED,
                    et::CHARGE_DISPUTE_FUNDS_WITHDRAWN,
                    et::CHARGE_DISPUTE_FUNDS_REINSTATED,
                ]);
            }
        }
    }
    events.sort_unstable();
    events.dedup();
    events.into_iter().map(String::from).collect()
}

fn dispute_event(d: stripe_client::webhook::StripeDispute) -> DisputeEvent {
    DisputeEvent {
        external_dispute_id: d.id,
        external_transaction_id: d.payment_intent.unwrap_or(d.charge),
        amount_minor: d.amount,
        currency: d.currency,
        failure: dispute_failure(&d.reason),
        reason: Some(d.reason),
    }
}

/// Dispute reasons that are the payer's claim: the invoice stays paid.
const DISPUTE_REASONS: &[&str] = &[
    "check_returned",
    "credit_not_processed",
    "customer_initiated",
    "debit_not_authorized",
    "duplicate",
    "fraudulent",
    "general",
    "noncompliant",
    "product_not_received",
    "product_unacceptable",
    "subscription_canceled",
    "unrecognized",
];

/// A bank debit (ACH, Bacs, SEPA) that failed after it succeeded arrives as a dispute with one of
/// these reasons: a failed collection, so the invoice reopens. An unknown reason is a dispute.
pub(crate) fn dispute_failure(reason: &str) -> Option<ReversalKind> {
    match reason {
        "insufficient_funds" => Some(ReversalKind::InsufficientFunds),
        "incorrect_account_details" | "bank_cannot_process" => Some(ReversalKind::Returned),
        known if DISPUTE_REASONS.contains(&known) => None,
        unknown => {
            log::warn!(
                "Stripe dispute reason '{unknown}' is not classified: treated as a dispute, the invoice stays paid"
            );
            None
        }
    }
}

fn payment_method_update(
    pm: stripe_client::payment_methods::PaymentMethod,
) -> PaymentMethodUpdatedEvent {
    let (card_brand, card_last4, card_exp_month, card_exp_year) = match &pm.card {
        Some(card) => (
            Some(card.brand.clone()),
            card.last4.clone(),
            Some(card.exp_month),
            Some(card.exp_year),
        ),
        None => (None, None, None, None),
    };
    PaymentMethodUpdatedEvent {
        external_payment_method_id: pm.id,
        card_brand,
        card_last4,
        card_exp_month,
        card_exp_year,
    }
}

// Webhook ingestion is the one production-critical path the integration suite
// doesn't cover; these pin signature verification + event normalization.

#[cfg(test)]
mod tests {
    #[test]
    fn bank_returns_reported_as_disputes_are_failed_collections() {
        use super::dispute_failure;
        use crate::domain::enums::ReversalKind;
        assert_eq!(
            dispute_failure("insufficient_funds"),
            Some(ReversalKind::InsufficientFunds)
        );
        assert_eq!(
            dispute_failure("incorrect_account_details"),
            Some(ReversalKind::Returned)
        );
        assert_eq!(
            dispute_failure("bank_cannot_process"),
            Some(ReversalKind::Returned)
        );
        assert_eq!(dispute_failure("fraudulent"), None);
        assert_eq!(dispute_failure("debit_not_authorized"), None);
        assert_eq!(dispute_failure("a_reason_from_tomorrow"), None);
    }

    #[test]
    fn stripe_methods_follow_the_dashboard_settings_then_capabilities() {
        use super::{MethodState, PaymentMethodTypeEnum, stripe_methods};
        use std::collections::HashMap;

        let config: stripe_client::payment_method_configurations::PaymentMethodConfiguration =
            serde_json::from_value(serde_json::json!({
                "id": "pmc_1", "active": true, "is_default": true, "parent": null,
                "card": {"available": true, "display_preference": {"value": "on"}},
                "sepa_debit": {"available": true, "display_preference": {"value": "on"}},
                "us_bank_account": {"available": true, "display_preference": {"value": "off"}},
                "bacs_debit": {"available": false, "display_preference": {"value": "on"}}
            }))
            .unwrap();
        let state = |methods: &[super::PaymentMethodAvailability], m: PaymentMethodTypeEnum| {
            methods.iter().find(|a| a.method == m).unwrap().state
        };

        // Direct account: no capabilities, the settings decide.
        let direct = stripe_methods(&HashMap::new(), Some(&config));
        assert_eq!(
            state(&direct, PaymentMethodTypeEnum::Card),
            MethodState::Enabled
        );
        assert_eq!(
            state(&direct, PaymentMethodTypeEnum::DirectDebitAch),
            MethodState::Unavailable
        );
        assert_eq!(
            state(&direct, PaymentMethodTypeEnum::DirectDebitBacs),
            MethodState::Unavailable
        );

        // No readable settings: explicit capabilities narrow, unreported rails stay enabled.
        let caps = HashMap::from([
            ("sepa_debit_payments".to_string(), "pending".to_string()),
            ("bacs_debit_payments".to_string(), "inactive".to_string()),
        ]);
        let connect = stripe_methods(&caps, None);
        assert_eq!(
            state(&connect, PaymentMethodTypeEnum::DirectDebitSepa),
            MethodState::Pending
        );
        assert_eq!(
            state(&connect, PaymentMethodTypeEnum::DirectDebitBacs),
            MethodState::Unavailable
        );
        assert_eq!(
            state(&connect, PaymentMethodTypeEnum::DirectDebitAch),
            MethodState::Enabled
        );
    }

    use super::*;
    use crate::adapters::payment::events::NormalizedEventKind;
    use crate::domain::connectors::Connector;
    use crate::domain::connectors::{
        ConnectorConfig, CredentialState, PaymentConnectorConfig, PaymentProviderConfig,
    };

    use chrono::NaiveDateTime;
    use common_domain::ids::{ConnectorId, TenantId};
    use hmac::{Hmac, KeyInit, Mac};
    use http::HeaderMap;
    use sha2::Sha256;

    const TEST_WEBHOOK_SECRET: &str = "whsec_test_super_secret_for_unit_tests";

    fn test_connector() -> Connector {
        Connector {
            status: crate::domain::enums::ConnectorStatusEnum::Active,
            credentials: CredentialState::Unverified,
            webhook_route: None,
            id: ConnectorId::new(),
            created_at: NaiveDateTime::default(),
            tenant_id: TenantId::new(),
            alias: "stripe-test".into(),
            config: ConnectorConfig::Payment(PaymentConnectorConfig::configured(
                PaymentProviderConfig::Stripe(test_public_data(), test_sensitive_data()),
            )),
        }
    }

    fn test_public_data() -> StripePublicData {
        StripePublicData {
            api_publishable_key: "pk_test_x".into(),
            account_id: Some("acct_test_x".into()),
            webhook_events_fingerprint: None,
        }
    }

    fn test_sensitive_data() -> StripeSensitiveData {
        StripeSensitiveData {
            api_secret_key: "sk_test_x".into(),
            webhook_secret: TEST_WEBHOOK_SECRET.into(),
            webhook_endpoint_id: None,
        }
    }

    /// The adapter under test, bound to the fixture connector.
    fn test_adapter() -> StripeConnector {
        let fixture = test_connector();
        StripeConnector::new(
            ConnectorRef::from(&fixture),
            test_public_data(),
            test_sensitive_data(),
        )
    }

    fn sign_payload(payload: &str, secret: &str, timestamp: i64) -> String {
        let signed = format!("{}.{}", timestamp, payload);
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("valid hmac key");
        mac.update(signed.as_bytes());
        let sig = mac.finalize().into_bytes();
        format!("t={},v1={}", timestamp, hex::encode(sig))
    }

    fn header_with_signature(sig: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("Stripe-Signature", sig.parse().unwrap());
        headers
    }

    /// Production contract: a correctly-signed, fresh payload must verify.
    #[test]
    fn verify_signature_accepts_valid() {
        let adapter = test_adapter();
        let payload = br#"{"id":"evt_1","type":"payment_intent.succeeded","data":{"object":{"object":"payment_intent","id":"pi_1","amount":1000,"amount_received":1000,"currency":"usd","livemode":false,"status":"succeeded","metadata":{}}}}"#;
        let sig = sign_payload(
            std::str::from_utf8(payload).unwrap(),
            TEST_WEBHOOK_SECRET,
            common_domain::clock::now().timestamp(),
        );
        let headers = header_with_signature(&sig);
        let secret = SecretString::from(TEST_WEBHOOK_SECRET.to_string());

        let result = adapter.verify_signature(payload, &headers, &secret);
        assert!(result.is_ok(), "valid signature must verify: {result:?}");
    }

    /// Production contract: payload signed with the *wrong* secret must be rejected.
    /// A bug here = accepting forged webhooks.
    #[test]
    fn verify_signature_rejects_wrong_secret() {
        let adapter = test_adapter();
        let payload = br#"{"id":"evt_2","type":"x"}"#;
        let sig = sign_payload(
            std::str::from_utf8(payload).unwrap(),
            "whsec_wrong_secret",
            common_domain::clock::now().timestamp(),
        );
        let headers = header_with_signature(&sig);
        let secret = SecretString::from(TEST_WEBHOOK_SECRET.to_string());

        let result = adapter.verify_signature(payload, &headers, &secret);
        assert!(result.is_err(), "wrong-secret signature must be rejected");
    }

    /// Production contract: a correctly-signed but *stale* payload (outside the
    /// 300s replay tolerance) must be rejected. Prevents replay attacks.
    #[test]
    fn verify_signature_rejects_stale_timestamp() {
        let adapter = test_adapter();
        let payload = br#"{"id":"evt_3","type":"x"}"#;
        let one_hour_ago = common_domain::clock::now().timestamp() - 3600;
        let sig = sign_payload(
            std::str::from_utf8(payload).unwrap(),
            TEST_WEBHOOK_SECRET,
            one_hour_ago,
        );
        let headers = header_with_signature(&sig);
        let secret = SecretString::from(TEST_WEBHOOK_SECRET.to_string());

        let result = adapter.verify_signature(payload, &headers, &secret);
        assert!(result.is_err(), "stale signature must be rejected");
    }

    /// Production contract: missing signature header = explicit error, not a panic.
    #[test]
    fn verify_signature_rejects_missing_header() {
        let adapter = test_adapter();
        let payload = br#"{"id":"evt_4","type":"x"}"#;
        let headers = HeaderMap::new();
        let secret = SecretString::from(TEST_WEBHOOK_SECRET.to_string());

        let result = adapter.verify_signature(payload, &headers, &secret);
        assert!(result.is_err(), "missing signature must be rejected");
    }

    /// Normalization: a `payment_intent.succeeded` event surfaces as
    /// `PaymentSucceeded` with the right transaction id pulled from metadata.
    #[test]
    fn parse_event_normalizes_payment_intent_succeeded() {
        let adapter = test_adapter();
        let payload = br#"{
            "id":"evt_succeeded_1",
            "type":"payment_intent.succeeded",
            "data":{"object":{
                "object":"payment_intent",
                "id":"pi_succeeded_1",
                "amount":5000,
                "amount_received":5000,
                "currency":"eur",
                "livemode":false,
                "status":"succeeded",
                "metadata":{
                    "meteroid.tenant_id":"tenant_abc",
                    "meteroid.transaction_id":"tx_xyz"
                }
            }}
        }"#;

        let parsed = adapter
            .parse_event(payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        assert_eq!(parsed.provider_event_id, "evt_succeeded_1");
        assert_eq!(parsed.provider_event_type, "payment_intent.succeeded");
        match parsed.kind {
            NormalizedEventKind::PaymentSucceeded(e) => {
                assert_eq!(e.external_transaction_id, "pi_succeeded_1");
                assert_eq!(e.amount_received_minor, 5000);
                assert_eq!(e.currency, "eur");
                assert_eq!(
                    e.meteroid_transaction_id.as_deref(),
                    Some("tx_xyz"),
                    "metadata extraction must surface our internal id"
                );
            }
            other => panic!("expected PaymentSucceeded, got {:?}", other),
        }
    }

    /// Normalization: `payment_intent.payment_failed` event surfaces as
    /// `PaymentFailed`, preserving the error message + meteroid metadata.
    #[test]
    fn parse_event_normalizes_payment_intent_failed() {
        let adapter = test_adapter();
        let payload = br#"{
            "id":"evt_failed_1",
            "type":"payment_intent.payment_failed",
            "data":{"object":{
                "object":"payment_intent",
                "id":"pi_failed_1",
                "amount":1000,
                "currency":"usd",
                "livemode":false,
                "status":"failed",
                "last_payment_error":{
                    "type":"card_error",
                    "code":"card_declined",
                    "decline_code":"insufficient_funds",
                    "message":"Your card has insufficient funds."
                },
                "metadata":{"meteroid.transaction_id":"tx_failed"}
            }}
        }"#;

        let parsed = adapter
            .parse_event(payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        match parsed.kind {
            NormalizedEventKind::PaymentFailed(e) => {
                assert_eq!(e.external_transaction_id, "pi_failed_1");
                assert!(e.message.contains("insufficient funds"));
                assert_eq!(e.code.as_deref(), Some("card_declined"));
                assert_eq!(e.meteroid_transaction_id.as_deref(), Some("tx_failed"));
            }
            other => panic!("expected PaymentFailed, got {:?}", other),
        }
    }

    /// Pins `payment_intent.requires_action` normalization (redirect_to_url shape).
    #[test]
    fn parse_event_normalizes_payment_intent_requires_action() {
        let adapter = test_adapter();
        let payload = br#"{
            "id":"evt_action_1",
            "type":"payment_intent.requires_action",
            "data":{"object":{
                "object":"payment_intent",
                "id":"pi_action_1",
                "amount":2000,
                "currency":"usd",
                "livemode":false,
                "status":"requires_action",
                "next_action":{
                    "type":"redirect_to_url",
                    "redirect_to_url":{
                        "url":"https://hooks.stripe.com/3d_secure/abc",
                        "return_url":"https://app.example.com/return"
                    }
                },
                "metadata":{"meteroid.transaction_id":"tx_action"}
            }}
        }"#;

        let parsed = adapter
            .parse_event(payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        match parsed.kind {
            NormalizedEventKind::PaymentRequiresAction(e) => {
                assert_eq!(e.external_transaction_id, "pi_action_1");
                assert_eq!(
                    e.action_url.as_deref(),
                    Some("https://hooks.stripe.com/3d_secure/abc"),
                    "URL must be extracted from next_action.redirect_to_url"
                );
                assert_eq!(e.meteroid_transaction_id.as_deref(), Some("tx_action"));
            }
            other => panic!("expected PaymentRequiresAction, got {:?}", other),
        }
    }

    fn test_intent(status: StripePaymentStatus) -> StripePaymentIntent {
        StripePaymentIntent {
            id: "pi_capture_1".to_string(),
            amount: 1500,
            amount_received: None,
            currency: "usd".to_string(),
            next_action: None,
            livemode: false,
            client_secret: None,
            status,
            last_payment_error: None,
            metadata: HashMap::new(),
        }
    }

    /// An authorized-but-uncaptured intent is not a failure: retrying while the
    /// auth stands risks a second authorization on the same card, so this must
    /// map to `Pending` (settlement pending), never `Failed`.
    #[test]
    fn intent_to_outcome_maps_requires_capture_to_pending() {
        let outcome = intent_to_outcome(
            test_intent(StripePaymentStatus::RequiresCapture),
            "pk_test_x",
        );
        assert!(
            matches!(outcome, ChargeOutcome::Pending(_)),
            "requires_capture must map to Pending, got {:?}",
            outcome
        );
    }

    /// `requires_payment_method` / `requires_confirmation` stay `Failed` and
    /// `retryable=false`: the idempotency key is derived from the transaction
    /// id, so a naive automatic retry would just replay the same cached
    /// Stripe response instead of making progress.
    #[test]
    fn intent_to_outcome_requires_payment_method_and_confirmation_not_retryable() {
        for status in [
            StripePaymentStatus::RequiresPaymentMethod,
            StripePaymentStatus::RequiresConfirmation,
        ] {
            let outcome = intent_to_outcome(test_intent(status.clone()), "pk_test_x");
            match outcome {
                ChargeOutcome::Failed(f) => {
                    assert!(!f.retryable, "{:?} must not be retryable", status);
                }
                other => panic!("expected Failed for {:?}, got {:?}", status, other),
            }
        }
    }

    /// Reconciliation must be able to conclude failure: a PaymentIntent has no
    /// `failed` status, so a declined attempt surfaces as
    /// `requires_payment_method`/`requires_confirmation` with
    /// `last_payment_error` set. Mapping those to `Pending` would poll forever
    /// and never fail the transaction (the exact case the reconcile worker
    /// exists for when the `payment_intent.payment_failed` webhook is lost).
    #[test]
    fn remote_status_maps_attempted_and_failed_intent_to_failed() {
        for status in [
            StripePaymentStatus::RequiresPaymentMethod,
            StripePaymentStatus::RequiresConfirmation,
        ] {
            let mut intent = test_intent(status.clone());
            intent.last_payment_error = Some(StripePaymentError {
                code: Some("card_declined".to_string()),
                decline_code: Some("insufficient_funds".to_string()),
                advice_code: None,
                message: Some("Your card has insufficient funds.".to_string()),
                error_type: Some("card_error".to_string()),
            });
            match remote_status_from_intent(intent) {
                RemoteTransactionStatus::Failed {
                    code,
                    message,
                    decline_kind,
                } => {
                    assert_eq!(code.as_deref(), Some("card_declined"), "{:?}", status);
                    assert_eq!(message, "Your card has insufficient funds.");
                    assert!(matches!(decline_kind, DeclineKind::InsufficientFunds));
                }
                other => panic!(
                    "expected Failed for attempted {:?}, got {:?}",
                    status, other
                ),
            }
        }
    }

    /// A pristine intent (no attempt yet — e.g. a portal checkout PI awaiting
    /// the customer) sits in `requires_payment_method`/`requires_confirmation`
    /// with no `last_payment_error`; reconciliation must keep it `Pending`,
    /// never kill an in-flight checkout. `requires_action` stays `Pending`
    /// even with a prior error (the customer is mid-3DS).
    #[test]
    fn remote_status_keeps_pristine_and_in_action_intents_pending() {
        for status in [
            StripePaymentStatus::RequiresPaymentMethod,
            StripePaymentStatus::RequiresConfirmation,
        ] {
            let outcome = remote_status_from_intent(test_intent(status.clone()));
            assert!(
                matches!(outcome, RemoteTransactionStatus::Pending),
                "pristine {:?} must stay Pending, got {:?}",
                status,
                outcome
            );
        }

        let mut in_action = test_intent(StripePaymentStatus::RequiresCustomerAction);
        in_action.last_payment_error = Some(StripePaymentError {
            message: Some("previous attempt failed".to_string()),
            ..Default::default()
        });
        let outcome = remote_status_from_intent(in_action);
        assert!(
            matches!(outcome, RemoteTransactionStatus::Pending),
            "requires_action must stay Pending, got {:?}",
            outcome
        );
    }

    /// `charge.refunded` event surfaces as `PaymentRefunded` with the refund
    /// id + amount, plus the parent PaymentIntent id (which is how we look up
    /// our local transaction).
    #[test]
    fn parse_event_normalizes_charge_refunded() {
        let adapter = test_adapter();
        let payload = br#"{
            "id":"evt_refund_1",
            "type":"charge.refunded",
            "data":{"object":{
                "object":"charge",
                "id":"ch_1",
                "payment_intent":"pi_refund_parent",
                "amount":2000,
                "amount_refunded":500,
                "currency":"usd",
                "refunds":{
                    "data":[{
                        "id":"re_1",
                        "amount":500,
                        "currency":"usd",
                        "status":"succeeded",
                        "payment_intent":"pi_refund_parent",
                        "charge":"ch_1"
                    }]
                }
            }}
        }"#;

        let parsed = adapter
            .parse_event(payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        match parsed.kind {
            NormalizedEventKind::PaymentRefunded(e) => {
                assert_eq!(e.external_transaction_id, "pi_refund_parent");
                assert_eq!(e.external_refund_id, "re_1");
                assert_eq!(e.amount_refunded_minor, 500);
                assert_eq!(e.currency, "usd");
                assert_eq!(e.refunds.len(), 1);
                assert_eq!(e.refunds[0].external_refund_id, "re_1");
                assert_eq!(e.refunds[0].amount_minor, Some(500));
                assert_eq!(e.refunds[0].status, ObservedRefundStatus::Succeeded);
            }
            other => panic!("expected PaymentRefunded, got {:?}", other),
        }
    }

    /// On a *second* partial refund, `amount_refunded_minor` must carry the
    /// charge's cumulative total (`charge.amount_refunded`), not the latest
    /// refund's own amount — the two differ once more than one refund exists.
    #[test]
    fn parse_event_normalizes_charge_refunded_cumulative_amount() {
        let adapter = test_adapter();
        let payload = br#"{
            "id":"evt_refund_2",
            "type":"charge.refunded",
            "data":{"object":{
                "object":"charge",
                "id":"ch_2",
                "payment_intent":"pi_refund_parent_2",
                "amount":2000,
                "amount_refunded":800,
                "currency":"usd",
                "refunds":{
                    "data":[
                        {
                            "id":"re_1",
                            "amount":500,
                            "currency":"usd",
                            "status":"succeeded",
                            "payment_intent":"pi_refund_parent_2",
                            "charge":"ch_2"
                        },
                        {
                            "id":"re_2",
                            "amount":300,
                            "currency":"usd",
                            "status":"succeeded",
                            "payment_intent":"pi_refund_parent_2",
                            "charge":"ch_2"
                        }
                    ]
                }
            }}
        }"#;

        let parsed = adapter
            .parse_event(payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        match parsed.kind {
            NormalizedEventKind::PaymentRefunded(e) => {
                assert_eq!(e.external_transaction_id, "pi_refund_parent_2");
                assert_eq!(
                    e.external_refund_id, "re_2",
                    "refund id should identify the most recent refund"
                );
                assert_eq!(
                    e.amount_refunded_minor, 800,
                    "amount must be the charge's cumulative amount_refunded (500+300), not the last refund's own amount (300)"
                );
                assert_eq!(
                    e.refunds
                        .iter()
                        .map(|r| (r.external_refund_id.as_str(), r.amount_minor))
                        .collect::<Vec<_>>(),
                    vec![("re_1", Some(500)), ("re_2", Some(300))],
                    "every refund on the wire must be carried so the handler can match what it knows"
                );
            }
            other => panic!("expected PaymentRefunded, got {:?}", other),
        }
    }

    /// `charge.refunded` with no `refunds.data` (e.g. an expanded-field miss)
    /// still falls back to the charge's cumulative `amount_refunded`.
    #[test]
    fn parse_event_normalizes_charge_refunded_no_refund_list() {
        let adapter = test_adapter();
        let payload = br#"{
            "id":"evt_refund_3",
            "type":"charge.refunded",
            "data":{"object":{
                "object":"charge",
                "id":"ch_3",
                "payment_intent":"pi_refund_parent_3",
                "amount":2000,
                "amount_refunded":2000,
                "currency":"usd"
            }}
        }"#;

        let parsed = adapter
            .parse_event(payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        match parsed.kind {
            NormalizedEventKind::PaymentRefunded(e) => {
                assert_eq!(e.external_refund_id, "");
                assert_eq!(e.amount_refunded_minor, 2000);
                assert!(
                    e.refunds.is_empty(),
                    "no refunds.data means nothing to match; only the cumulative total is known"
                );
            }
            other => panic!("expected PaymentRefunded, got {:?}", other),
        }
    }

    /// Same cumulative-amount contract as above, but against the real
    /// provider fixture (two partial refunds, 3000+2000 => 5000) instead of
    /// a hand-written payload.
    #[test]
    fn parse_event_normalizes_charge_refunded_real_fixture() {
        let adapter = test_adapter();
        let payload = include_str!(
            "../../../../../tests/integration/fixtures/webhooks/stripe/charge.refunded.json"
        );

        let parsed = adapter
            .parse_event(payload.as_bytes(), &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        match parsed.kind {
            NormalizedEventKind::PaymentRefunded(e) => {
                assert_eq!(e.external_transaction_id, "pi_test_129");
                assert_eq!(e.external_refund_id, "re_test_2");
                assert_eq!(
                    e.amount_refunded_minor, 5000,
                    "must be the charge's cumulative amount_refunded (3000+2000), not the last refund's own amount (2000)"
                );
                assert_eq!(e.currency, "usd");
                assert_eq!(
                    e.refunds
                        .iter()
                        .map(|r| (r.external_refund_id.as_str(), r.amount_minor, r.status))
                        .collect::<Vec<_>>(),
                    vec![
                        ("re_test_1", Some(3000), ObservedRefundStatus::Succeeded),
                        ("re_test_2", Some(2000), ObservedRefundStatus::Succeeded)
                    ]
                );
            }
            other => panic!("expected PaymentRefunded, got {:?}", other),
        }
    }

    /// A `charge.refunded` whose list holds a failed refund must not count it as
    /// succeeded — the handler would otherwise settle a REFUND row that never paid.
    #[test]
    fn parse_event_normalizes_charge_refunded_with_a_failed_refund() {
        let adapter = test_adapter();
        let payload = br#"{
            "id":"evt_refund_4",
            "type":"charge.refunded",
            "data":{"object":{
                "object":"charge",
                "id":"ch_4",
                "payment_intent":"pi_refund_parent_4",
                "amount":2000,
                "amount_refunded":700,
                "currency":"usd",
                "refunds":{
                    "data":[
                        {
                            "id":"re_ok",
                            "amount":700,
                            "currency":"usd",
                            "status":"succeeded",
                            "payment_intent":"pi_refund_parent_4",
                            "charge":"ch_4"
                        },
                        {
                            "id":"re_ko",
                            "amount":300,
                            "currency":"usd",
                            "status":"failed",
                            "failure_reason":"declined",
                            "payment_intent":"pi_refund_parent_4",
                            "charge":"ch_4"
                        }
                    ]
                }
            }}
        }"#;

        let parsed = adapter
            .parse_event(payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        match parsed.kind {
            NormalizedEventKind::PaymentRefunded(e) => {
                assert_eq!(
                    e.refunds
                        .iter()
                        .map(|r| (r.external_refund_id.as_str(), r.status))
                        .collect::<Vec<_>>(),
                    vec![
                        ("re_ok", ObservedRefundStatus::Succeeded),
                        ("re_ko", ObservedRefundStatus::Failed)
                    ]
                );
                assert_eq!(
                    e.amount_refunded_minor, 700,
                    "the failed refund is excluded from the charge's cumulative total"
                );
            }
            other => panic!("expected PaymentRefunded, got {:?}", other),
        }
    }

    /// `refund.updated` settles the REFUND row it names; the parent PaymentIntent
    /// is carried so the handler can fall back to the parent when the id is unknown.
    #[test]
    fn parse_event_normalizes_refund_updated_succeeded() {
        let adapter = test_adapter();
        let payload = br#"{
            "id":"evt_re_updated_1",
            "type":"refund.updated",
            "data":{"object":{
                "object":"refund",
                "id":"re_updated_1",
                "amount":1250,
                "currency":"eur",
                "status":"succeeded",
                "payment_intent":"pi_updated_1",
                "charge":"ch_updated_1",
                "metadata":{"meteroid.tenant_id":"ten_abc"}
            }}
        }"#;

        let parsed = adapter
            .parse_event(payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        assert_eq!(parsed.owner_tenant_id.as_deref(), Some("ten_abc"));
        match parsed.kind {
            NormalizedEventKind::RefundStatusChanged(e) => {
                assert_eq!(e.external_refund_id, "re_updated_1");
                assert_eq!(e.external_transaction_id.as_deref(), Some("pi_updated_1"));
                assert_eq!(e.status, RemoteRefundStatus::Succeeded);
                assert_eq!(e.amount_minor, Some(1250));
            }
            other => panic!("expected RefundStatusChanged, got {:?}", other),
        }
    }

    /// Stripe's legacy alias for the same object; a failed refund must carry its
    /// `failure_reason` so the row records why it did not pay.
    #[test]
    fn parse_event_normalizes_charge_refund_updated_failed() {
        let adapter = test_adapter();
        let payload = br#"{
            "id":"evt_re_updated_2",
            "type":"charge.refund.updated",
            "data":{"object":{
                "object":"refund",
                "id":"re_updated_2",
                "amount":400,
                "currency":"usd",
                "status":"failed",
                "failure_reason":"lost_or_stolen_card",
                "charge":"ch_updated_2"
            }}
        }"#;

        let parsed = adapter
            .parse_event(payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        match parsed.kind {
            NormalizedEventKind::RefundStatusChanged(e) => {
                assert_eq!(e.external_refund_id, "re_updated_2");
                assert_eq!(
                    e.external_transaction_id.as_deref(),
                    Some("ch_updated_2"),
                    "a refund with no payment_intent falls back to its charge"
                );
                assert_eq!(
                    e.status,
                    RemoteRefundStatus::Failed {
                        code: Some("lost_or_stolen_card".to_string()),
                        message: "lost_or_stolen_card".to_string(),
                    }
                );
            }
            other => panic!("expected RefundStatusChanged, got {:?}", other),
        }
    }

    /// An in-flight refund must stay Pending: settling or failing it on a guess
    /// would either double-count the refund or wrongly free the amount again.
    #[test]
    fn parse_event_keeps_pending_and_unknown_refund_statuses_pending() {
        let adapter = test_adapter();
        let status_of = |status: &str| {
            let payload = format!(
                r#"{{"id":"evt_re_p","type":"refund.updated","data":{{"object":{{
                    "object":"refund","id":"re_p","amount":100,"currency":"usd",
                    "status":"{status}","payment_intent":"pi_p"
                }}}}}}"#
            );
            match adapter
                .parse_event(payload.as_bytes(), &HeaderMap::new())
                .expect("parse should succeed")
                .expect("event should be normalized")
                .kind
            {
                NormalizedEventKind::RefundStatusChanged(e) => e.status,
                other => panic!("expected RefundStatusChanged, got {:?}", other),
            }
        };

        assert_eq!(status_of("pending"), RemoteRefundStatus::Pending);
        assert_eq!(status_of("requires_action"), RemoteRefundStatus::Pending);
        assert_eq!(status_of("something_new"), RemoteRefundStatus::Pending);
        assert_eq!(status_of("canceled"), RemoteRefundStatus::Canceled);
    }

    fn test_refund(status: &str) -> StripeRefund {
        serde_json::from_str(&format!(
            r#"{{"id":"re_out","object":"refund","amount":900,"created":1721728695,
                "currency":"usd","status":"{status}","payment_intent":"pi_out"}}"#
        ))
        .expect("parse refund fixture")
    }

    /// A canceled refund never moved money, so it is a failure like a decline —
    /// leaving it Pending would keep the amount locked out of `refundable_amount`.
    #[test]
    fn refund_outcome_maps_stripe_statuses() {
        match refund_to_outcome(test_refund("succeeded")) {
            RefundOutcome::Succeeded(r) => {
                assert_eq!(r.external_refund_id, "re_out");
                assert_eq!(r.amount_refunded_minor, 900);
                assert_eq!(
                    r.processed_at,
                    DateTime::from_timestamp(1721728695, 0).unwrap().naive_utc(),
                    "Stripe's own timestamp, not wall-clock"
                );
            }
            other => panic!("expected Succeeded, got {other:?}"),
        }
        assert!(matches!(
            refund_to_outcome(test_refund("pending")),
            RefundOutcome::Pending(_)
        ));
        assert!(matches!(
            refund_to_outcome(test_refund("failed")),
            RefundOutcome::Failed(_)
        ));
        match refund_to_outcome(test_refund("canceled")) {
            RefundOutcome::Failed(f) => assert_eq!(f.code.as_deref(), Some("canceled")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// Stripe reports the cumulative refunded total on the charge, never on a
    /// refund, so the snapshot must leave it unset rather than echo this amount.
    #[test]
    fn refund_snapshot_leaves_the_cumulative_total_unknown() {
        let snapshot = refund_snapshot(test_refund("succeeded"));
        assert_eq!(snapshot.external_refund_id, "re_out");
        assert_eq!(snapshot.external_transaction_id, "pi_out");
        assert_eq!(snapshot.amount_minor, 900);
        assert_eq!(snapshot.currency, "usd");
        assert_eq!(snapshot.status, RemoteRefundStatus::Succeeded);
        assert_eq!(snapshot.cumulative_refunded_minor, None);
    }

    /// Whatever `normalize_kind` parses must be registered, or Stripe never sends it.
    #[test]
    fn refund_subscription_registers_every_parsed_refund_event() {
        let events = subscriptions_to_stripe_events(&[NormalizedEventSubscription::Refunds]);
        assert!(events.contains(&event_type::CHARGE_REFUNDED.to_string()));
        assert!(events.contains(&event_type::REFUND_UPDATED.to_string()));
        assert!(events.contains(&event_type::CHARGE_REFUND_UPDATED.to_string()));
    }

    /// `charge.dispute.created` surfaces as `DisputeOpened` so the handler can
    /// flag the invoice for support attention.
    #[test]
    fn parse_event_normalizes_dispute_created() {
        let adapter = test_adapter();
        let payload = br#"{
            "id":"evt_dispute_1",
            "type":"charge.dispute.created",
            "data":{"object":{
                "object":"dispute",
                "id":"dp_1",
                "charge":"ch_1",
                "payment_intent":"pi_disputed",
                "amount":2000,
                "currency":"usd",
                "reason":"fraudulent",
                "status":"needs_response"
            }}
        }"#;

        let parsed = adapter
            .parse_event(payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        match parsed.kind {
            NormalizedEventKind::DisputeOpened(e) => {
                assert_eq!(e.external_dispute_id, "dp_1");
                assert_eq!(e.external_transaction_id, "pi_disputed");
                assert_eq!(e.amount_minor, 2000);
                assert_eq!(e.reason.as_deref(), Some("fraudulent"));
            }
            other => panic!("expected DisputeOpened, got {:?}", other),
        }
    }

    /// `charge.dispute.funds_withdrawn` on the real provider fixture surfaces as
    /// `DisputeFundsWithdrawn` — the event that actually moves money, so the
    /// handler reverses the payment and reopens the invoice.
    #[test]
    fn parse_event_normalizes_dispute_funds_withdrawn_real_fixture() {
        let adapter = test_adapter();
        let payload = include_str!(
            "../../../../../tests/integration/fixtures/webhooks/stripe/charge.dispute.funds_withdrawn.json"
        );

        let parsed = adapter
            .parse_event(payload.as_bytes(), &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        match parsed.kind {
            NormalizedEventKind::DisputeFundsWithdrawn(e) => {
                assert_eq!(e.external_transaction_id, "pi_test_disputed_1");
                assert_eq!(e.amount_minor, 10000);
            }
            other => panic!("expected DisputeFundsWithdrawn, got {:?}", other),
        }
    }

    /// `charge.dispute.closed` with status=lost surfaces as `DisputeLost`
    /// (the event that triggers invoice locking).
    #[test]
    fn parse_event_normalizes_dispute_closed_lost() {
        let adapter = test_adapter();
        let payload = br#"{
            "id":"evt_dispute_lost",
            "type":"charge.dispute.closed",
            "data":{"object":{
                "object":"dispute",
                "id":"dp_lost",
                "charge":"ch_2",
                "payment_intent":"pi_disputed_2",
                "amount":1000,
                "currency":"usd",
                "reason":"fraudulent",
                "status":"lost"
            }}
        }"#;

        let parsed = adapter
            .parse_event(payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        assert!(
            matches!(parsed.kind, NormalizedEventKind::DisputeLost(_)),
            "lost dispute must surface as DisputeLost so handler can lock invoice"
        );
    }

    /// `payment_method.automatically_updated` (card-expiring async flow) —
    /// Stripe's account-updater rotated the customer's card details on our
    /// behalf and we need to keep our snapshot in sync.
    #[test]
    fn parse_event_normalizes_payment_method_auto_updated() {
        let adapter = test_adapter();
        let payload = br#"{
            "id":"evt_pm_auto",
            "type":"payment_method.automatically_updated",
            "data":{"object":{
                "object":"payment_method",
                "id":"pm_updated",
                "type":"card",
                "card":{
                    "brand":"visa",
                    "last4":"4242",
                    "exp_month":12,
                    "exp_year":2030,
                    "country":"US"
                }
            }}
        }"#;

        let parsed = adapter
            .parse_event(payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        match parsed.kind {
            NormalizedEventKind::PaymentMethodUpdated(e) => {
                assert_eq!(e.external_payment_method_id, "pm_updated");
                assert_eq!(e.card_last4.as_deref(), Some("4242"));
                assert_eq!(e.card_exp_year, Some(2030));
            }
            other => panic!("expected PaymentMethodUpdated, got {:?}", other),
        }
    }

    /// Normalization: `setup_intent.succeeded` surfaces as `PaymentMethodAttached`
    /// with the customer / connection metadata preserved so the handler can
    /// look up our internal records.
    #[test]
    fn parse_event_normalizes_setup_intent_succeeded() {
        let adapter = test_adapter();
        let payload = br#"{
            "id":"evt_setup_1",
            "type":"setup_intent.succeeded",
            "data":{"object":{
                "object":"setup_intent",
                "id":"seti_1",
                "client_secret":"seti_secret_x",
                "created":1700000000,
                "customer":"cus_xyz",
                "payment_method":"pm_card_visa",
                "livemode":false,
                "payment_method_types":["card"],
                "status":"succeeded",
                "usage":"off_session",
                "metadata":{
                    "meteroid.connection_id":"conn_123",
                    "meteroid.customer_id":"cust_456"
                }
            }}
        }"#;

        let parsed = adapter
            .parse_event(payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        match parsed.kind {
            NormalizedEventKind::PaymentMethodAttached(e) => {
                assert_eq!(e.external_payment_method_id, "pm_card_visa");
                assert_eq!(e.external_customer_id, "cus_xyz");
                assert_eq!(e.meteroid_connection_id.as_deref(), Some("conn_123"));
                assert_eq!(e.meteroid_customer_id.as_deref(), Some("cust_456"));
            }
            other => panic!("expected PaymentMethodAttached, got {:?}", other),
        }
    }

    /// An event carrying an object type we don't model (here `customer.created`)
    /// must decode without error and normalize to `Acknowledged` so the pgmq
    /// message is acked instead of retried forever. Stripe delivers such events
    /// whenever the endpoint is registered for more than our payment set.
    #[test]
    fn parse_event_tolerates_unknown_object_type() {
        let adapter = test_adapter();
        let payload = br#"{
            "id":"evt_customer_created_1",
            "type":"customer.created",
            "data":{"object":{
                "object":"customer",
                "id":"cus_unknown_1",
                "livemode":false,
                "metadata":{}
            }}
        }"#;

        let parsed = adapter
            .parse_event(payload, &HeaderMap::new())
            .expect("parse must not error on an unmodeled object type")
            .expect("event should still normalize");

        assert_eq!(parsed.provider_event_type, "customer.created");
        assert!(
            matches!(parsed.kind, NormalizedEventKind::Acknowledged { .. }),
            "unmodeled object types must be acknowledged, got {:?}",
            parsed.kind
        );
    }

    fn stripe_error(status: u16, body: serde_json::Value) -> stripe_client::error::RequestError {
        let mut err: stripe_client::error::RequestError =
            serde_json::from_value(body["error"].clone()).unwrap();
        err.http_status = status;
        err
    }

    /// A confirm-time 402 is recorded as a declined attempt, with the intent Stripe created.
    #[test]
    fn card_decline_is_a_failed_charge_not_an_error() {
        let err = stripe_error(
            402,
            serde_json::json!({"error": {
                "type": "card_error", "code": "card_declined", "decline_code": "insufficient_funds",
                "message": "Your card has insufficient funds.",
                "payment_intent": {"id": "pi_123", "object": "payment_intent", "status": "requires_payment_method"}
            }}),
        );
        let failure = super::card_error_to_failure(err);
        assert_eq!(failure.external_id.as_deref(), Some("pi_123"));
        assert_eq!(failure.code.as_deref(), Some("insufficient_funds"));
        assert_eq!(
            failure.decline_kind,
            crate::adapters::payment::model::DeclineKind::InsufficientFunds
        );
        assert!(!failure.retryable);
    }

    /// Network "do not reattempt" answers are hard declines, so dunning never re-presents them.
    #[test]
    fn do_not_retry_declines_are_hard() {
        use crate::adapters::payment::model::DeclineKind;
        let advised = stripe_error(
            402,
            serde_json::json!({"error": {
                "type": "card_error", "code": "card_declined", "decline_code": "generic_decline",
                "advice_code": "do_not_try_again", "message": "Your card was declined."
            }}),
        );
        assert_eq!(
            super::card_error_to_failure(advised).decline_kind,
            DeclineKind::DoNotRetry
        );
        let revoked = stripe_error(
            402,
            serde_json::json!({"error": {
                "type": "card_error", "code": "card_declined",
                "decline_code": "revocation_of_authorization", "message": "Declined."
            }}),
        );
        let kind = super::card_error_to_failure(revoked).decline_kind;
        assert_eq!(kind, DeclineKind::DoNotRetry);
        assert!(kind.is_hard());
    }

    /// An idempotency conflict says nothing about the original charge: never a decline.
    #[test]
    fn idempotency_conflicts_map_to_transport() {
        use super::{ConnectorError, StripeError, map_stripe_error};
        for (status, error_type) in [(409, "idempotency_error"), (400, "idempotency_error")] {
            let err = stripe_error(
                status,
                serde_json::json!({"error": {"type": error_type, "message": "Keys for idempotent requests can only be used with the same parameters"}}),
            );
            let report = map_stripe_error(StripeError::Stripe(err));
            assert!(
                matches!(report.current_context(), ConnectorError::Transport(_)),
                "{status} {error_type} must be Transport, got {:?}",
                report.current_context()
            );
        }
        let declined = stripe_error(
            400,
            serde_json::json!({"error": {"type": "invalid_request_error", "message": "bad"}}),
        );
        assert!(matches!(
            map_stripe_error(StripeError::Stripe(declined)).current_context(),
            ConnectorError::Charge(_)
        ));
    }
}
