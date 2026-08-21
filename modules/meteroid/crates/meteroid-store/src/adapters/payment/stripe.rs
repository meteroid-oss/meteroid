//! Stripe implementation of the [`super::PaymentConnector`] trait family.
//! All Stripe-specific type mapping lives here; the rest of the codebase only
//! sees the normalized types from [`super::model`] and [`super::events`].

use super::connector::{
    ConnectorCapabilities, ConnectorIdentity, CustomerOps, MandateOps, MandateSetupMode,
    PaymentOps, ReconcileOps, RefundOps, WebhookOps,
};
use super::error::ConnectorError;
use super::events::{
    DisputeEvent, NormalizedEventKind, NormalizedEventSubscription, NormalizedWebhookEvent,
    PaymentFailedEvent, PaymentMethodAttachedEvent, PaymentMethodDetachedEvent,
    PaymentMethodUpdatedEvent, PaymentPendingEvent, PaymentRefundedEvent,
    PaymentRequiresActionEvent, PaymentSucceededEvent,
};
use super::model::{
    ChargeAcknowledged, ChargeCancelled, ChargeFailure, ChargeOutcome, ChargeReceipt,
    ChargeRequest, CreateCustomerRequest, DeclineKind, ExternalCustomerRef,
    MandateSetupInstruction, MandateSetupRequest, PaymentMethodSnapshot, RefundOutcome,
    RefundRequest, RefundSnapshot, RegisteredWebhook, RemoteTransactionStatus,
};
use crate::domain::connectors::{Connector, ProviderData, ProviderSensitiveData};
use crate::domain::enums::ConnectorProviderEnum;
use crate::domain::{Address, Customer, CustomerConnection, PaymentMethodTypeEnum};
use async_trait::async_trait;
use chrono::DateTime;
use common_domain::ids::BaseId;
use error_stack::Report;
use http::HeaderMap;
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::sync::OnceLock;
use stripe_client::client::StripeClient;
use stripe_client::customers::{
    CreateCustomer, CustomerApi, CustomerShipping, OptionalFieldsAddress,
};
use stripe_client::error::StripeError;
use stripe_client::payment_intents::{
    PaymentIntentApi, PaymentIntentRequest, StripeNextAction, StripePaymentError,
    StripePaymentIntent, StripePaymentStatus,
};
use stripe_client::payment_methods::{
    PaymentMethod, PaymentMethodsApi, StripePaymentMethodType as StripePmType,
};
use stripe_client::setup_intents::{
    CreateSetupIntent, CreateSetupIntentUsage, SetupIntentApi,
    StripePaymentMethodType as StripeSetupPmType,
};
use stripe_client::webhook::{Event, EventObject, StripeWebhook, event_type};
use stripe_client::webhook_endpoints::{
    CreateWebhookEndpointRequest, UpdateWebhookEndpointRequest, WebhookEndpointApi,
};

/// The bits describe Stripe's *protocol* capability, gated by impl state:
/// `supports_refunds` is `false` because `refund()` still returns
/// `Unsupported` — flip it once that's implemented. (Webhook
/// self-registration, by contrast, is implemented and auto-invoked on
/// connect.)
const STRIPE_CAPABILITIES: ConnectorCapabilities = ConnectorCapabilities {
    supports_cards: true,
    supports_mandates: true,
    supports_refunds: false,
    // Implied by `supports_refunds` (see `assert_capabilities_consistent`
    // in contract.rs): can't do partial refunds when `refund()` does none.
    supports_partial_refunds: false,
    supports_3ds: true,
    supports_disputes: true,
    supports_self_webhook_registration: true,
    asynchronous_settlement: true,
    supported_payment_methods: &[
        PaymentMethodTypeEnum::Card,
        PaymentMethodTypeEnum::DirectDebitSepa,
        PaymentMethodTypeEnum::DirectDebitAch,
        PaymentMethodTypeEnum::DirectDebitBacs,
    ],
    mandate_setup_mode: MandateSetupMode::EmbeddedClientSecret,
    webhook_replay_tolerance_secs: 300,
};

/// Wraps a process-wide [`StripeClient`]: every tenant shares one pool since
/// they all hit `api.stripe.com`. Per-tenant data lives on [`Connector`].
#[derive(Debug, Clone, Copy)]
pub struct StripeConnector;

impl StripeConnector {
    pub fn new() -> Self {
        StripeConnector
    }

    /// Process-wide pooled client; the wrapper holds tuning (timeouts, retry
    /// strategy) we want fixed at process startup.
    fn client() -> &'static StripeClient {
        static CLIENT: OnceLock<StripeClient> = OnceLock::new();
        CLIENT.get_or_init(StripeClient::new)
    }
}

impl Default for StripeConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectorIdentity for StripeConnector {
    fn provider(&self) -> ConnectorProviderEnum {
        ConnectorProviderEnum::Stripe
    }

    fn capabilities(&self) -> &ConnectorCapabilities {
        &STRIPE_CAPABILITIES
    }
}

#[async_trait]
impl CustomerOps for StripeConnector {
    async fn create_customer(
        &self,
        connector: &Connector,
        customer: &Customer,
        request: CreateCustomerRequest,
    ) -> Result<ExternalCustomerRef, Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;

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
                    name: Some(customer.name.clone()),
                    address: customer.billing_address.as_ref().map(map_address),
                    email: customer.billing_email.clone(),
                    source: None,
                    shipping: customer
                        .shipping_address
                        .as_ref()
                        .and_then(|a| a.address.as_ref())
                        .map(|a| CustomerShipping {
                            address: map_address(a),
                            name: customer.name.clone(),
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
        connector: &Connector,
        connection: &CustomerConnection,
        request: MandateSetupRequest<'_>,
    ) -> Result<MandateSetupInstruction, Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;
        let publishable_key = extract_publishable_key(connector)?;

        let stripe_payment_methods: Vec<StripeSetupPmType> = request
            .payment_methods
            .iter()
            .filter_map(to_stripe_setup_pm_type)
            .collect();

        let metadata = HashMap::from([
            (
                "meteroid.tenant_id".to_string(),
                connector.tenant_id.as_base62(),
            ),
            (
                "meteroid.customer_id".to_string(),
                connection.customer_id.as_base62(),
            ),
            (
                "meteroid.connection_id".to_string(),
                connection.id.as_base62(),
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
        connector: &Connector,
        external_payment_method_id: &str,
        external_customer_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;

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
        _connector: &Connector,
        _intent_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        // Stripe SetupIntents finalize client-side and confirm via the
        // `setup_intent.succeeded` webhook; no server-side complete step.
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Stripe,
            capability: "mandate.complete (Stripe finalizes client-side via webhook)",
        }))
    }
}

#[async_trait]
impl PaymentOps for StripeConnector {
    async fn charge_off_session(
        &self,
        connector: &Connector,
        request: ChargeRequest<'_>,
    ) -> Result<ChargeOutcome, Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;
        // Non-secret; surfaced in next_action so the portal can init Stripe.js.
        let publishable_key = extract_publishable_key(connector)
            .map(|k| k.expose_secret().to_string())
            .unwrap_or_default();

        let metadata = HashMap::from([
            (
                "meteroid.tenant_id".to_string(),
                connector.tenant_id.as_base62(),
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
            Err(e) => Err(map_stripe_error(e)),
        }
    }
}

#[async_trait]
impl RefundOps for StripeConnector {
    async fn refund(
        &self,
        _connector: &Connector,
        _request: RefundRequest<'_>,
    ) -> Result<RefundOutcome, Report<ConnectorError>> {
        // Refunds not implemented.
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Stripe,
            capability: "refund",
        }))
    }

    async fn fetch_refund(
        &self,
        _connector: &Connector,
        _external_refund_id: &str,
    ) -> Result<RefundSnapshot, Report<ConnectorError>> {
        // `charge.refunded` already carries the cumulative amount inline.
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Stripe,
            capability: "fetch_refund",
        }))
    }
}

#[async_trait]
impl ReconcileOps for StripeConnector {
    async fn fetch_transaction_status(
        &self,
        connector: &Connector,
        external_transaction_id: &str,
    ) -> Result<RemoteTransactionStatus, Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;

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
            // `StripePaymentIntent` doesn't carry Stripe's `created` timestamp
            // (stripe-client crate); wall-clock is a stand-in until it's added.
            processed_at: chrono::Utc::now().naive_utc(),
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
        | StripePaymentStatus::RequiresCustomerAction
        | StripePaymentStatus::RequiresPaymentMethod
        | StripePaymentStatus::RequiresConfirmation
        | StripePaymentStatus::RequiresCapture => RemoteTransactionStatus::Pending,
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
        connector: &Connector,
        url: &str,
        subscriptions: &[NormalizedEventSubscription],
    ) -> Result<RegisteredWebhook, Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;
        let enabled_events = subscriptions_to_stripe_events(subscriptions);

        let endpoint = Self::client()
            .create_webhook_endpoint(
                CreateWebhookEndpointRequest {
                    url: url.to_string(),
                    enabled_events,
                    description: Some(format!("Meteroid (tenant {})", connector.tenant_id)),
                },
                &secret_key,
                // Idempotency key must be stable across retries: keyed on
                // (tenant_id, alias, url), never `connector.id` (freshly
                // generated per call) — an unstable key orphans an endpoint per retry.
                format!(
                    "webhook_register:{}:{}:{}",
                    connector.tenant_id, connector.alias, url
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

    async fn unregister_webhook(
        &self,
        connector: &Connector,
        endpoint_id: &str,
    ) -> Result<(), Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;

        Self::client()
            .delete_webhook_endpoint(endpoint_id, &secret_key)
            .await
            .map(|_| ())
            .map_err(map_stripe_error)
    }

    async fn sync_webhook_events(
        &self,
        connector: &Connector,
        endpoint_id: &str,
        subscriptions: &[NormalizedEventSubscription],
    ) -> Result<(), Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;
        let enabled_events = subscriptions_to_stripe_events(subscriptions);

        Self::client()
            .update_webhook_endpoint(
                endpoint_id,
                UpdateWebhookEndpointRequest { enabled_events },
                &secret_key,
                // Stable across retries; `endpoint_id` is provider-side and persistent.
                format!("webhook_sync:{}:{}", connector.tenant_id, endpoint_id),
            )
            .await
            .map(|_| ())
            .map_err(map_stripe_error)
    }

    fn verify_signature(
        &self,
        _connector: &Connector,
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
        _connector: &Connector,
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

fn extract_secret_key(connector: &Connector) -> Result<SecretString, Report<ConnectorError>> {
    match &connector.sensitive {
        Some(ProviderSensitiveData::Stripe(data)) => {
            Ok(SecretString::from(data.api_secret_key.clone()))
        }
        Some(_) => Err(Report::new(ConnectorError::Configuration(
            "connector is not a stripe connector".to_string(),
        ))),
        None => Err(Report::new(ConnectorError::Configuration(
            "stripe connector has no api_secret_key".to_string(),
        ))),
    }
}

fn extract_publishable_key(connector: &Connector) -> Result<SecretString, Report<ConnectorError>> {
    match &connector.data {
        Some(ProviderData::Stripe(data)) => {
            Ok(SecretString::from(data.api_publishable_key.clone()))
        }
        Some(_) => Err(Report::new(ConnectorError::Configuration(
            "connector is not a stripe connector".to_string(),
        ))),
        None => Err(Report::new(ConnectorError::Configuration(
            "stripe connector has no api_publishable_key".to_string(),
        ))),
    }
}

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

    PaymentMethodSnapshot {
        external_payment_method_id: method.id,
        payment_method_type,
        account_number_hint,
        card_brand,
        card_last4,
        card_exp_month,
        card_exp_year,
        // Stripe events carry our metadata directly.
        meteroid_connection_id: None,
        meteroid_customer_id: None,
        // Stripe invoice payments charge inline (no post-mandate webhook charge).
        meteroid_invoice_id: None,
        // Hosted combined mandate+payment checkout is GoCardless-only.
        meteroid_checkout_session_id: None,
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
            processed_at: chrono::Utc::now().naive_utc(),
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
        // 3DS/SCA: `next_action` says what the client must do. See requires_action_outcome.
        StripePaymentStatus::RequiresCustomerAction => requires_action_outcome(
            intent.id,
            intent.next_action,
            intent.client_secret,
            publishable_key,
        ),
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
fn requires_action_outcome(
    intent_id: String,
    next_action: Option<StripeNextAction>,
    client_secret: Option<String>,
    publishable_key: &str,
) -> ChargeOutcome {
    use super::model::RequiresActionInstruction;
    // A bare redirect (rare for cards) can be opened directly. Everything else
    // is SDK-driven: the portal calls Stripe.js `handleNextAction` with the
    // PaymentIntent client_secret (returned on create), so carry it through.
    let redirect_url = next_action
        .as_ref()
        .and_then(|a| a.redirect_to_url.as_ref())
        .and_then(|r| r.url.clone());

    let instruction = match redirect_url {
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
    };
    ChargeOutcome::RequiresAction(instruction)
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
    let Some(decline_code) = err
        .as_ref()
        .and_then(|e| e.decline_code.as_deref().or(e.code.as_deref()))
    else {
        return DeclineKind::Other;
    };
    match decline_code {
        "insufficient_funds" => DeclineKind::InsufficientFunds,
        "do_not_honor" | "generic_decline" => DeclineKind::DoNotHonor,
        "expired_card" | "incorrect_cvc" | "incorrect_number" => DeclineKind::CardExpired,
        "authentication_required" => DeclineKind::AuthenticationRequired,
        "fraudulent" | "stolen_card" | "lost_card" => DeclineKind::Fraud,
        "processing_error" | "issuer_not_available" | "try_again_later" => {
            DeclineKind::ProcessingError
        }
        _ => DeclineKind::Other,
    }
}

/// Network failures and Stripe 5xx map to `Transport` (safe to retry with the
/// same idempotency key); 4xx map to `Charge`/`Configuration` (not retryable).
fn map_stripe_error(e: StripeError) -> Report<ConnectorError> {
    match e {
        StripeError::Timeout => Report::new(ConnectorError::Transport("stripe timeout".into())),
        StripeError::ClientError(msg) => Report::new(ConnectorError::Transport(msg)),
        StripeError::Stripe(req_err) if req_err.http_status >= 500 => Report::new(
            ConnectorError::Transport(format!("stripe 5xx: {}", req_err)),
        ),
        StripeError::Stripe(req_err) => Report::new(ConnectorError::Charge(format!(
            "stripe rejected: {} (code={:?})",
            req_err.message.unwrap_or_default(),
            req_err.code
        ))),
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
        .unwrap_or_else(chrono::Utc::now);
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
            // `amount_refunded_minor` is always the charge's cumulative
            // refunded total (`charge.amount_refunded`), never a single
            // refund's delta — unambiguous for a phase-2 reversal consumer
            // that needs to reconcile against the charge, not sum deltas.
            NormalizedEventKind::PaymentRefunded(PaymentRefundedEvent {
                external_transaction_id: parent_pi,
                external_refund_id: refund_id,
                amount_refunded_minor: charge.amount_refunded,
                currency: charge.currency,
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
                events.push(et::CHARGE_REFUNDED);
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
        reason: Some(d.reason),
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
    use super::*;
    use crate::adapters::payment::events::NormalizedEventKind;
    use crate::domain::connectors::{Connector, StripeSensitiveData};
    use crate::domain::enums::{ConnectorProviderEnum, ConnectorTypeEnum};
    use chrono::NaiveDateTime;
    use common_domain::ids::{ConnectorId, TenantId};
    use hmac::{Hmac, KeyInit, Mac};
    use http::HeaderMap;
    use sha2::Sha256;

    const TEST_WEBHOOK_SECRET: &str = "whsec_test_super_secret_for_unit_tests";

    fn test_connector() -> Connector {
        Connector {
            id: ConnectorId::new(),
            created_at: NaiveDateTime::default(),
            tenant_id: TenantId::new(),
            alias: "stripe-test".into(),
            connector_type: ConnectorTypeEnum::PaymentProvider,
            provider: ConnectorProviderEnum::Stripe,
            data: None,
            sensitive: Some(crate::domain::connectors::ProviderSensitiveData::Stripe(
                StripeSensitiveData {
                    api_secret_key: "sk_test_x".into(),
                    webhook_secret: TEST_WEBHOOK_SECRET.into(),
                    webhook_endpoint_id: None,
                },
            )),
        }
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
        let connector = test_connector();
        let payload = br#"{"id":"evt_1","type":"payment_intent.succeeded","data":{"object":{"object":"payment_intent","id":"pi_1","amount":1000,"amount_received":1000,"currency":"usd","livemode":false,"status":"succeeded","metadata":{}}}}"#;
        let sig = sign_payload(
            std::str::from_utf8(payload).unwrap(),
            TEST_WEBHOOK_SECRET,
            chrono::Utc::now().timestamp(),
        );
        let headers = header_with_signature(&sig);
        let secret = SecretString::from(TEST_WEBHOOK_SECRET.to_string());

        let result =
            StripeConnector::new().verify_signature(&connector, payload, &headers, &secret);
        assert!(result.is_ok(), "valid signature must verify: {result:?}");
    }

    /// Production contract: payload signed with the *wrong* secret must be rejected.
    /// A bug here = accepting forged webhooks.
    #[test]
    fn verify_signature_rejects_wrong_secret() {
        let connector = test_connector();
        let payload = br#"{"id":"evt_2","type":"x"}"#;
        let sig = sign_payload(
            std::str::from_utf8(payload).unwrap(),
            "whsec_wrong_secret",
            chrono::Utc::now().timestamp(),
        );
        let headers = header_with_signature(&sig);
        let secret = SecretString::from(TEST_WEBHOOK_SECRET.to_string());

        let result =
            StripeConnector::new().verify_signature(&connector, payload, &headers, &secret);
        assert!(result.is_err(), "wrong-secret signature must be rejected");
    }

    /// Production contract: a correctly-signed but *stale* payload (outside the
    /// 300s replay tolerance) must be rejected. Prevents replay attacks.
    #[test]
    fn verify_signature_rejects_stale_timestamp() {
        let connector = test_connector();
        let payload = br#"{"id":"evt_3","type":"x"}"#;
        let one_hour_ago = chrono::Utc::now().timestamp() - 3600;
        let sig = sign_payload(
            std::str::from_utf8(payload).unwrap(),
            TEST_WEBHOOK_SECRET,
            one_hour_ago,
        );
        let headers = header_with_signature(&sig);
        let secret = SecretString::from(TEST_WEBHOOK_SECRET.to_string());

        let result =
            StripeConnector::new().verify_signature(&connector, payload, &headers, &secret);
        assert!(result.is_err(), "stale signature must be rejected");
    }

    /// Production contract: missing signature header = explicit error, not a panic.
    #[test]
    fn verify_signature_rejects_missing_header() {
        let connector = test_connector();
        let payload = br#"{"id":"evt_4","type":"x"}"#;
        let headers = HeaderMap::new();
        let secret = SecretString::from(TEST_WEBHOOK_SECRET.to_string());

        let result =
            StripeConnector::new().verify_signature(&connector, payload, &headers, &secret);
        assert!(result.is_err(), "missing signature must be rejected");
    }

    /// Normalization: a `payment_intent.succeeded` event surfaces as
    /// `PaymentSucceeded` with the right transaction id pulled from metadata.
    #[test]
    fn parse_event_normalizes_payment_intent_succeeded() {
        let connector = test_connector();
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

        let parsed = StripeConnector::new()
            .parse_event(&connector, payload, &HeaderMap::new())
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
        let connector = test_connector();
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

        let parsed = StripeConnector::new()
            .parse_event(&connector, payload, &HeaderMap::new())
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
        let connector = test_connector();
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

        let parsed = StripeConnector::new()
            .parse_event(&connector, payload, &HeaderMap::new())
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
        let connector = test_connector();
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

        let parsed = StripeConnector::new()
            .parse_event(&connector, payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        match parsed.kind {
            NormalizedEventKind::PaymentRefunded(e) => {
                assert_eq!(e.external_transaction_id, "pi_refund_parent");
                assert_eq!(e.external_refund_id, "re_1");
                assert_eq!(e.amount_refunded_minor, 500);
                assert_eq!(e.currency, "usd");
            }
            other => panic!("expected PaymentRefunded, got {:?}", other),
        }
    }

    /// On a *second* partial refund, `amount_refunded_minor` must carry the
    /// charge's cumulative total (`charge.amount_refunded`), not the latest
    /// refund's own amount — the two differ once more than one refund exists.
    #[test]
    fn parse_event_normalizes_charge_refunded_cumulative_amount() {
        let connector = test_connector();
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

        let parsed = StripeConnector::new()
            .parse_event(&connector, payload, &HeaderMap::new())
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
            }
            other => panic!("expected PaymentRefunded, got {:?}", other),
        }
    }

    /// `charge.refunded` with no `refunds.data` (e.g. an expanded-field miss)
    /// still falls back to the charge's cumulative `amount_refunded`.
    #[test]
    fn parse_event_normalizes_charge_refunded_no_refund_list() {
        let connector = test_connector();
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

        let parsed = StripeConnector::new()
            .parse_event(&connector, payload, &HeaderMap::new())
            .expect("parse should succeed")
            .expect("event should be normalized");

        match parsed.kind {
            NormalizedEventKind::PaymentRefunded(e) => {
                assert_eq!(e.external_refund_id, "");
                assert_eq!(e.amount_refunded_minor, 2000);
            }
            other => panic!("expected PaymentRefunded, got {:?}", other),
        }
    }

    /// Same cumulative-amount contract as above, but against the real
    /// provider fixture (two partial refunds, 3000+2000 => 5000) instead of
    /// a hand-written payload.
    #[test]
    fn parse_event_normalizes_charge_refunded_real_fixture() {
        let connector = test_connector();
        let payload = include_str!(
            "../../../../../tests/integration/fixtures/webhooks/stripe/charge.refunded.json"
        );

        let parsed = StripeConnector::new()
            .parse_event(&connector, payload.as_bytes(), &HeaderMap::new())
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
            }
            other => panic!("expected PaymentRefunded, got {:?}", other),
        }
    }

    /// `charge.dispute.created` surfaces as `DisputeOpened` so the handler can
    /// flag the invoice for support attention.
    #[test]
    fn parse_event_normalizes_dispute_created() {
        let connector = test_connector();
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

        let parsed = StripeConnector::new()
            .parse_event(&connector, payload, &HeaderMap::new())
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
        let connector = test_connector();
        let payload = include_str!(
            "../../../../../tests/integration/fixtures/webhooks/stripe/charge.dispute.funds_withdrawn.json"
        );

        let parsed = StripeConnector::new()
            .parse_event(&connector, payload.as_bytes(), &HeaderMap::new())
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
        let connector = test_connector();
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

        let parsed = StripeConnector::new()
            .parse_event(&connector, payload, &HeaderMap::new())
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
        let connector = test_connector();
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

        let parsed = StripeConnector::new()
            .parse_event(&connector, payload, &HeaderMap::new())
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
        let connector = test_connector();
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

        let parsed = StripeConnector::new()
            .parse_event(&connector, payload, &HeaderMap::new())
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
        let connector = test_connector();
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

        let parsed = StripeConnector::new()
            .parse_event(&connector, payload, &HeaderMap::new())
            .expect("parse must not error on an unmodeled object type")
            .expect("event should still normalize");

        assert_eq!(parsed.provider_event_type, "customer.created");
        assert!(
            matches!(parsed.kind, NormalizedEventKind::Acknowledged { .. }),
            "unmodeled object types must be acknowledged, got {:?}",
            parsed.kind
        );
    }
}
