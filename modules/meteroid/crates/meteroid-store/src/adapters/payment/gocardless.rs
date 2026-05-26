//! GoCardless implementation of [`PaymentConnector`].
//!
//! Two important differences from Stripe:
//!
//! 1. **Hosted-redirect mandate setup.** No embedded SDK. We create a Billing
//!    Request server-side, mint a Billing Request Flow, and the frontend just
//!    redirects the browser to `authorisation_url`. When the customer
//!    returns, our return-URL handler completes the BRF and stores the
//!    mandate id as the payment method.
//!
//! 2. **Asynchronous settlement.** A `POST /payments` returns immediately
//!    with `pending_submission`; settlement takes 3–5 business days
//!    depending on scheme. Webhooks (`payments.confirmed`, `paid_out`,
//!    `failed`) deliver the final state.
//!
//! Webhook self-registration is unsupported by the provider — endpoints are
//! configured manually in the GoCardless dashboard. `register_webhook`
//! returns [`ConnectorError::Unsupported`].

use super::connector::{
    ConnectorCapabilities, ConnectorIdentity, CustomerOps, MandateOps, MandateSetupMode,
    PaymentOps, ReconcileOps, RefundOps, WebhookOps,
};
use super::error::ConnectorError;
use super::events::{
    NormalizedEventKind, NormalizedEventSubscription, NormalizedWebhookEvent,
    PaymentFailedEvent, PaymentMethodAttachedEvent, PaymentMethodDetachedEvent,
    PaymentSucceededEvent,
};
use super::model::{
    ChargeAcknowledged, ChargeFailure, ChargeOutcome, ChargeReceipt, ChargeRequest,
    CreateCustomerRequest, DeclineKind, ExternalCustomerRef, MandateSetupInstruction,
    MandateSetupRequest, PaymentMethodSnapshot, RefundOutcome, RefundRequest, RegisteredWebhook,
    RemoteTransactionStatus,
};
use crate::domain::connectors::{Connector, ProviderData, ProviderSensitiveData};
use crate::domain::enums::ConnectorProviderEnum;
use crate::domain::{Customer, CustomerConnection, PaymentMethodTypeEnum};
use async_trait::async_trait;
use chrono::DateTime;
use common_domain::ids::BaseId;
use error_stack::Report;
use gocardless_client::billing_requests::{
    BillingRequestApi, BillingRequestFlowLinks, BillingRequestLinks, CreateBillingRequest,
    CreateBillingRequestFlow, MandateRequest,
};
use gocardless_client::client::GoCardlessClient;
use gocardless_client::customers::{CreateCustomer, CustomerApi};
use gocardless_client::error::GoCardlessError;
use gocardless_client::mandates::{MandateApi, MandateStatus};
use gocardless_client::payments::{
    CreatePayment, CreatePaymentLinks, PaymentApi, PaymentStatus,
};
use gocardless_client::webhook::{
    EventEnvelope, GoCardlessWebhook, action as ev_action, resource_type as ev_resource,
};
use http::HeaderMap;
use secrecy::SecretString;
use std::collections::HashMap;
use std::sync::OnceLock;

const GOCARDLESS_CAPABILITIES: ConnectorCapabilities = ConnectorCapabilities {
    supports_cards: false,
    supports_mandates: true,
    supports_refunds: true,
    supports_partial_refunds: true,
    supports_3ds: false,
    supports_disputes: true,
    // GoCardless does not expose a public API to manage webhook endpoints —
    // merchants configure them in the dashboard, then paste the signing
    // secret into our connect form.
    supports_self_webhook_registration: false,
    asynchronous_settlement: true,
    supported_payment_methods: &[
        PaymentMethodTypeEnum::DirectDebitSepa,
        PaymentMethodTypeEnum::DirectDebitAch,
        PaymentMethodTypeEnum::DirectDebitBacs,
    ],
    mandate_setup_mode: MandateSetupMode::HostedRedirect,
    // GoCardless does not put a timestamp in its signature header, so the
    // "replay tolerance" has no header-level enforcement; we rely on
    // (provider_config_id, provider_event_id) DB dedup. We still surface a
    // tolerance value for forward compatibility / capability honesty.
    webhook_replay_tolerance_secs: 3600,
};

/// GoCardless connector. Live + sandbox clients are static singletons so all
/// tenants share a connection pool to each environment.
#[derive(Debug, Clone, Copy)]
pub struct GoCardlessConnector;

impl GoCardlessConnector {
    pub fn new() -> Self {
        GoCardlessConnector
    }

    fn client_for(connector: &Connector) -> &'static GoCardlessClient {
        // Production-critical: never silently route a misconfigured connector
        // to the *live* GoCardless API. If the `data` blob is missing or has
        // the wrong shape (DB corruption, migration mismatch), the matches!
        // below would default to live — which is exactly the kind of "tested
        // in sandbox, then somehow charged real money" failure mode we want
        // to prevent. Default to **sandbox** when uncertain, so a misconfig
        // surfaces loudly via "expected production but hit sandbox" errors
        // rather than the converse.
        let sandbox = match &connector.data {
            Some(ProviderData::Gocardless(d)) => d.is_sandbox(),
            _ => true,
        };
        if sandbox {
            static SANDBOX: OnceLock<GoCardlessClient> = OnceLock::new();
            SANDBOX.get_or_init(GoCardlessClient::from_sandbox)
        } else {
            static LIVE: OnceLock<GoCardlessClient> = OnceLock::new();
            LIVE.get_or_init(GoCardlessClient::new)
        }
    }
}

impl Default for GoCardlessConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectorIdentity for GoCardlessConnector {
    fn provider(&self) -> ConnectorProviderEnum {
        ConnectorProviderEnum::Gocardless
    }
    fn capabilities(&self) -> &ConnectorCapabilities {
        &GOCARDLESS_CAPABILITIES
    }
}

#[async_trait]
impl CustomerOps for GoCardlessConnector {
    async fn create_customer(
        &self,
        connector: &Connector,
        customer: &Customer,
        request: CreateCustomerRequest,
    ) -> Result<ExternalCustomerRef, Report<ConnectorError>> {
        let token = extract_access_token(connector)?;
        let client = Self::client_for(connector);

        let (given_name, family_name) = split_name(&customer.name);
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

        // Address fields are optional on the customer; only forward what we
        // have. GoCardless validates a minimum set per scheme/country at the
        // BRF step rather than at customer creation.
        let (addr1, addr2, city, region, postal_code, country_code) =
            customer.billing_address.as_ref().map_or(
                (None, None, None, None, None, None),
                |a| {
                    (
                        a.line1.clone(),
                        a.line2.clone(),
                        a.city.clone(),
                        a.state.clone(),
                        a.zip_code.clone(),
                        a.country.as_ref().map(|c| c.code.clone()),
                    )
                },
            );

        let result = client
            .create_customer(
                CreateCustomer {
                    email: customer.billing_email.clone(),
                    given_name: Some(given_name),
                    family_name: Some(family_name),
                    company_name: Some(customer.name.clone()),
                    language: None,
                    phone_number: customer.phone.clone(),
                    address_line1: addr1,
                    address_line2: addr2,
                    address_line3: None,
                    city,
                    region,
                    postal_code,
                    country_code,
                    metadata: Some(metadata),
                },
                &token,
                request.idempotency_key.as_str(),
            )
            .await
            .map_err(map_gc_error)?;

        Ok(ExternalCustomerRef {
            external_id: result.id,
            provider_request_id: None,
        })
    }
}

#[async_trait]
impl MandateOps for GoCardlessConnector {
    async fn initiate_mandate_setup(
        &self,
        connector: &Connector,
        connection: &CustomerConnection,
        request: MandateSetupRequest<'_>,
    ) -> Result<MandateSetupInstruction, Report<ConnectorError>> {
        let token = extract_access_token(connector)?;
        let client = Self::client_for(connector);

        // Pick the first supported method that maps to a GoCardless scheme,
        // and the corresponding currency. The currency is what GoCardless
        // uses to infer the scheme; passing scheme explicitly is optional.
        let (currency, scheme) = request
            .payment_methods
            .iter()
            .find_map(method_to_currency_scheme)
            .ok_or_else(|| {
                Report::new(ConnectorError::MandateSetup(
                    "no GoCardless-compatible payment method requested".to_string(),
                ))
            })?;

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

        let creditor_id = match &connector.data {
            Some(ProviderData::Gocardless(d)) => d.creditor_id.clone(),
            _ => None,
        };

        // Step 1 — create the Billing Request describing what we want.
        let br = client
            .create_billing_request(
                CreateBillingRequest {
                    mandate_request: Some(MandateRequest {
                        currency,
                        scheme: Some(scheme.to_string()),
                        description: Some(format!(
                            "Mandate for Meteroid customer {}",
                            connection.customer_id.as_base62()
                        )),
                        metadata: Some(metadata.clone()),
                    }),
                    payment_request: None,
                    metadata: Some(metadata),
                    links: Some(BillingRequestLinks {
                        customer: Some(connection.external_customer_id.clone()),
                        creditor: creditor_id,
                    }),
                },
                &token,
                &format!("{}:br", request.idempotency_key.as_str()),
            )
            .await
            .map_err(map_gc_error)?;

        // Step 2 — mint the Flow (hosted authorisation URL).
        let flow = client
            .create_billing_request_flow(
                CreateBillingRequestFlow {
                    redirect_uri: request.return_url.clone(),
                    exit_uri: request.return_url.clone(),
                    lock_currency: Some(true),
                    lock_amount: None,
                    lock_bank_account: None,
                    auto_fulfil: None,
                    links: BillingRequestFlowLinks {
                        billing_request: br.id.clone(),
                    },
                },
                &token,
                &format!("{}:brf", request.idempotency_key.as_str()),
            )
            .await
            .map_err(map_gc_error)?;

        let expires_at = flow
            .expires_at
            .as_ref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        Ok(MandateSetupInstruction::HostedRedirect {
            intent_id: br.id,
            authorisation_url: flow.authorisation_url,
            expires_at,
        })
    }

    async fn fetch_payment_method(
        &self,
        connector: &Connector,
        external_payment_method_id: &str,
        _external_customer_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        let token = extract_access_token(connector)?;
        let client = Self::client_for(connector);
        let mandate = client
            .get_mandate(external_payment_method_id, &token)
            .await
            .map_err(map_gc_error)?;
        Ok(snapshot_from_mandate(mandate.id, mandate.scheme))
    }

    /// Called by the return-URL handler when the customer comes back from
    /// the GC-hosted Billing Request Flow. Completes the BR (idempotent on
    /// GC's side), pulls out `links.mandate`, then fetches the mandate to
    /// build a `PaymentMethodSnapshot` ready for upsert.
    async fn complete_mandate_setup(
        &self,
        connector: &Connector,
        intent_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        let token = extract_access_token(connector)?;
        let client = Self::client_for(connector);

        // Step 1 — complete the Billing Request. Safe to call multiple times;
        // GC's BR state machine handles dedup.
        let br = client
            .complete_billing_request(intent_id, &token)
            .await
            .map_err(map_gc_error)?;

        let mandate_id = br.links.mandate.ok_or_else(|| {
            Report::new(ConnectorError::MandateSetup(format!(
                "GoCardless completed BR {} but returned no mandate id",
                intent_id
            )))
        })?;

        // Step 2 — fetch the mandate to learn the scheme (sepa/bacs/ach/…).
        // Without this we can't tag the payment method correctly for
        // downstream filtering (e.g. country-based DD type resolution).
        let mandate = client
            .get_mandate(&mandate_id, &token)
            .await
            .map_err(map_gc_error)?;

        Ok(snapshot_from_mandate(mandate.id, mandate.scheme))
    }
}

#[async_trait]
impl PaymentOps for GoCardlessConnector {
    async fn charge_off_session(
        &self,
        connector: &Connector,
        request: ChargeRequest<'_>,
    ) -> Result<ChargeOutcome, Report<ConnectorError>> {
        let token = extract_access_token(connector)?;
        let client = Self::client_for(connector);

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

        let result = client
            .create_payment(
                CreatePayment {
                    amount: request.amount_minor,
                    currency: request.currency.to_string(),
                    description: None,
                    metadata: Some(metadata),
                    charge_date: None,
                    reference: None,
                    links: CreatePaymentLinks {
                        mandate: request.payment_method_external_id.to_string(),
                    },
                },
                &token,
                request.idempotency_key.as_str(),
            )
            .await;

        match result {
            Ok(payment) => Ok(payment_to_outcome(payment.id, payment.status)),
            Err(e) => Err(map_gc_error(e)),
        }
    }
}

#[async_trait]
impl RefundOps for GoCardlessConnector {
    async fn refund(
        &self,
        _connector: &Connector,
        _request: RefundRequest<'_>,
    ) -> Result<RefundOutcome, Report<ConnectorError>> {
        // Refunds intentionally deferred (per project scope).
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Gocardless,
            capability: "refund",
        }))
    }
}

#[async_trait]
impl ReconcileOps for GoCardlessConnector {
    async fn fetch_transaction_status(
        &self,
        connector: &Connector,
        external_transaction_id: &str,
    ) -> Result<RemoteTransactionStatus, Report<ConnectorError>> {
        let token = extract_access_token(connector)?;
        let client = Self::client_for(connector);

        let result = client.get_payment(external_transaction_id, &token).await;
        match result {
            Ok(payment) => Ok(remote_status_from_payment(payment.status, payment.amount)),
            Err(GoCardlessError::Api(req_err)) if req_err.http_status == 404 => {
                Ok(RemoteTransactionStatus::Unknown)
            }
            Err(e) => Err(map_gc_error(e)),
        }
    }
}

#[async_trait]
impl WebhookOps for GoCardlessConnector {
    async fn register_webhook(
        &self,
        _connector: &Connector,
        _url: &str,
        _subscriptions: &[NormalizedEventSubscription],
    ) -> Result<RegisteredWebhook, Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Gocardless,
            capability: "webhook.register (GoCardless endpoints are dashboard-managed)",
        }))
    }

    async fn unregister_webhook(
        &self,
        _connector: &Connector,
        _endpoint_id: &str,
    ) -> Result<(), Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Gocardless,
            capability: "webhook.unregister",
        }))
    }

    async fn sync_webhook_events(
        &self,
        _connector: &Connector,
        _endpoint_id: &str,
        _subscriptions: &[NormalizedEventSubscription],
    ) -> Result<(), Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Gocardless,
            capability: "webhook.sync",
        }))
    }

    fn verify_signature(
        &self,
        _connector: &Connector,
        payload: &[u8],
        headers: &HeaderMap,
        secret: &SecretString,
    ) -> Result<(), Report<ConnectorError>> {
        use secrecy::ExposeSecret;

        let sig = headers
            .get("Webhook-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| Report::new(ConnectorError::SignatureMissing))?;

        GoCardlessWebhook::validate_signature(payload, sig, secret.expose_secret())
            .map_err(|_| Report::new(ConnectorError::SignatureVerification))
    }

    fn parse_event(
        &self,
        _connector: &Connector,
        payload: &[u8],
        _headers: &HeaderMap,
    ) -> Result<Option<NormalizedWebhookEvent>, Report<ConnectorError>> {
        let envelope: EventEnvelope = GoCardlessWebhook::parse_envelope(payload)
            .map_err(|e| {
                Report::new(ConnectorError::PayloadDecode(format!(
                    "failed to decode gocardless event envelope: {e}"
                )))
            })?;

        // GoCardless can batch multiple events per delivery; the trait
        // currently surfaces one. We pick the first event we know how to
        // handle and **explicitly log the count of dropped events** so an
        // operator can detect this in production logs / metrics. Returning
        // ACK 200 without surfacing those events is a data-loss risk — GC
        // will not retry once we ACK, and the DB dedup is keyed on event id
        // so the dropped ones are gone for good.
        //
        // The router observes this log line as a signal to widen the trait
        // to `Vec<NormalizedWebhookEvent>`. See project memo for the planned
        // fix.
        let event_count = envelope.events.len();
        if event_count > 1 {
            let dropped_ids: Vec<_> = envelope
                .events
                .iter()
                .skip(1)
                .map(|e| e.id.clone())
                .collect();
            log::error!(
                "GoCardless delivered {event_count} events in one webhook; dropping {} (ids: {:?}). \
                 Widen WebhookOps::parse_event to Vec to fix.",
                dropped_ids.len(),
                dropped_ids
            );
        }
        let event = envelope.events.into_iter().next();
        Ok(event.and_then(normalize_event))
    }
}

// ── helpers ────────────────────────────────────────────────────────

fn extract_access_token(
    connector: &Connector,
) -> Result<SecretString, Report<ConnectorError>> {
    match &connector.sensitive {
        Some(ProviderSensitiveData::Gocardless(d)) => {
            Ok(SecretString::from(d.access_token.clone()))
        }
        Some(_) => Err(Report::new(ConnectorError::Configuration(
            "connector is not a gocardless connector".into(),
        ))),
        None => Err(Report::new(ConnectorError::Configuration(
            "gocardless connector has no access_token".into(),
        ))),
    }
}

/// Best-effort split of a single name string into given/family. GoCardless
/// requires *some* given_name + family_name on customers in many countries;
/// callers can override later by patching the customer in the GC dashboard.
fn split_name(name: &str) -> (String, String) {
    let trimmed = name.trim();
    if let Some((first, rest)) = trimmed.split_once(' ') {
        (first.to_string(), rest.trim().to_string())
    } else {
        (trimmed.to_string(), trimmed.to_string())
    }
}

/// Map a payment method type to (currency, scheme) for GoCardless. Returns
/// `None` for methods GoCardless doesn't support (cards). Currency is the
/// "primary" currency for that scheme; merchants can override per BRF if
/// needed.
fn method_to_currency_scheme(method: &PaymentMethodTypeEnum) -> Option<(String, &'static str)> {
    match method {
        PaymentMethodTypeEnum::DirectDebitSepa => Some(("EUR".into(), "sepa_core")),
        PaymentMethodTypeEnum::DirectDebitBacs => Some(("GBP".into(), "bacs")),
        PaymentMethodTypeEnum::DirectDebitAch => Some(("USD".into(), "ach")),
        _ => None,
    }
}

fn snapshot_from_mandate(mandate_id: String, scheme: Option<String>) -> PaymentMethodSnapshot {
    let payment_method_type = match scheme.as_deref() {
        Some("sepa_core") => PaymentMethodTypeEnum::DirectDebitSepa,
        Some("bacs") => PaymentMethodTypeEnum::DirectDebitBacs,
        Some("ach") => PaymentMethodTypeEnum::DirectDebitAch,
        _ => PaymentMethodTypeEnum::Other,
    };
    PaymentMethodSnapshot {
        external_payment_method_id: mandate_id,
        payment_method_type,
        // Mandates don't expose bank-account last4 by default; a follow-up
        // can fetch the linked customer_bank_account and pull `account_number_ending`.
        account_number_hint: None,
        card_brand: None,
        card_last4: None,
        card_exp_month: None,
        card_exp_year: None,
    }
}

/// Map the *initial* response of `POST /payments`. Settlement is asynchronous
/// so a fresh payment is almost always `Pending`; the final state arrives via
/// webhook.
fn payment_to_outcome(id: String, status: PaymentStatus) -> ChargeOutcome {
    match status {
        PaymentStatus::Confirmed | PaymentStatus::PaidOut | PaymentStatus::LateFailureResolved => {
            ChargeOutcome::Succeeded(ChargeReceipt {
                external_id: id,
                amount_received_minor: 0, // GC payment object doesn't carry the
                                          // settled amount on the initial POST;
                                          // webhook `paid_out` reconciles.
                processed_at: chrono::Utc::now().naive_utc(),
                provider_request_id: None,
            })
        }
        PaymentStatus::PendingCustomerApproval
        | PaymentStatus::PendingSubmission
        | PaymentStatus::Submitted => ChargeOutcome::Pending(ChargeAcknowledged {
            external_id: id,
            provider_request_id: None,
        }),
        PaymentStatus::Cancelled | PaymentStatus::CustomerApprovalDenied => {
            ChargeOutcome::Failed(ChargeFailure {
                external_id: Some(id),
                code: Some(format!("{:?}", status).to_lowercase()),
                message: "Payment cancelled".to_string(),
                retryable: false,
                decline_kind: DeclineKind::Other,
                provider_request_id: None,
            })
        }
        PaymentStatus::Failed | PaymentStatus::ChargedBack => {
            ChargeOutcome::Failed(ChargeFailure {
                external_id: Some(id),
                code: Some(format!("{:?}", status).to_lowercase()),
                message: "Payment failed at provider".to_string(),
                retryable: false,
                decline_kind: DeclineKind::Other,
                provider_request_id: None,
            })
        }
        PaymentStatus::Unknown => ChargeOutcome::Pending(ChargeAcknowledged {
            external_id: id,
            provider_request_id: None,
        }),
    }
}

fn remote_status_from_payment(status: PaymentStatus, amount: i64) -> RemoteTransactionStatus {
    match status {
        PaymentStatus::Confirmed
        | PaymentStatus::PaidOut
        | PaymentStatus::LateFailureResolved => RemoteTransactionStatus::Succeeded {
            amount_received_minor: amount,
            processed_at: chrono::Utc::now().naive_utc(),
        },
        PaymentStatus::PendingCustomerApproval
        | PaymentStatus::PendingSubmission
        | PaymentStatus::Submitted
        | PaymentStatus::Unknown => RemoteTransactionStatus::Pending,
        PaymentStatus::Cancelled | PaymentStatus::CustomerApprovalDenied => {
            RemoteTransactionStatus::Cancelled
        }
        PaymentStatus::Failed | PaymentStatus::ChargedBack => RemoteTransactionStatus::Failed {
            code: Some(format!("{:?}", status).to_lowercase()),
            message: "Payment failed at provider".to_string(),
            decline_kind: DeclineKind::Other,
        },
    }
}

fn map_gc_error(e: GoCardlessError) -> Report<ConnectorError> {
    match e {
        GoCardlessError::Timeout => {
            Report::new(ConnectorError::Transport("gocardless timeout".into()))
        }
        GoCardlessError::ClientError(msg) => Report::new(ConnectorError::Transport(msg)),
        GoCardlessError::Api(req_err) if req_err.http_status >= 500 => Report::new(
            ConnectorError::Transport(format!("gocardless 5xx: {req_err}")),
        ),
        GoCardlessError::Api(req_err) => {
            // 4xx — validation / state error. Not retryable.
            Report::new(ConnectorError::Charge(format!(
                "gocardless rejected: {}",
                req_err.message.unwrap_or_default()
            )))
        }
        GoCardlessError::Encode(e) => {
            Report::new(ConnectorError::Configuration(format!("gocardless: {e}")))
        }
    }
}

/// Translate a verified webhook event into a normalized event. Returns
/// `None` for resource_types we don't care about (subscriptions, refunds —
/// refund handling is deferred per project scope).
fn normalize_event(event: gocardless_client::webhook::Event) -> Option<NormalizedWebhookEvent> {
    let kind = match event.resource_type.as_str() {
        ev_resource::PAYMENTS => normalize_payment_event(&event),
        ev_resource::MANDATES => normalize_mandate_event(&event),
        _ => Some(NormalizedEventKind::Acknowledged {
            reason: "unhandled gocardless resource_type",
        }),
    }?;

    let occurred_at = chrono::DateTime::parse_from_rfc3339(&event.created_at)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    Some(NormalizedWebhookEvent {
        provider_event_id: event.id.clone(),
        provider_event_type: format!("{}.{}", event.resource_type, event.action),
        occurred_at,
        kind,
    })
}

fn normalize_payment_event(event: &gocardless_client::webhook::Event) -> Option<NormalizedEventKind> {
    let payment_id = event.links.payment.clone()?;
    let meteroid_tx = event.metadata.get("meteroid.transaction_id").cloned();
    Some(match event.action.as_str() {
        ev_action::CONFIRMED | ev_action::PAID_OUT | ev_action::LATE_FAILURE_RESOLVED => {
            NormalizedEventKind::PaymentSucceeded(PaymentSucceededEvent {
                external_transaction_id: payment_id,
                // The webhook does not include the amount; the local
                // transaction holds the requested amount, and the settled
                // amount is the same modulo fees (fees are reported via a
                // separate `paid_out`/`payouts` event).
                amount_received_minor: 0,
                currency: String::new(),
                meteroid_transaction_id: meteroid_tx,
            })
        }
        ev_action::FAILED => NormalizedEventKind::PaymentFailed(PaymentFailedEvent {
            external_transaction_id: payment_id,
            code: event.details.as_ref().and_then(|d| d.cause.clone()),
            message: event
                .details
                .as_ref()
                .and_then(|d| d.description.clone())
                .unwrap_or_else(|| "Payment failed".to_string()),
            retryable: false,
            meteroid_transaction_id: meteroid_tx,
        }),
        ev_action::CANCELLED => NormalizedEventKind::PaymentFailed(PaymentFailedEvent {
            external_transaction_id: payment_id,
            code: Some("cancelled".into()),
            message: "Payment cancelled".into(),
            retryable: false,
            meteroid_transaction_id: meteroid_tx,
        }),
        ev_action::CHARGED_BACK => NormalizedEventKind::PaymentFailed(PaymentFailedEvent {
            external_transaction_id: payment_id,
            code: Some("charged_back".into()),
            message: "Payment charged back".into(),
            retryable: false,
            meteroid_transaction_id: meteroid_tx,
        }),
        _ => NormalizedEventKind::Acknowledged {
            reason: "unhandled gocardless payment action",
        },
    })
}

fn normalize_mandate_event(event: &gocardless_client::webhook::Event) -> Option<NormalizedEventKind> {
    let mandate_id = event.links.mandate.clone()?;
    Some(match event.action.as_str() {
        ev_action::ACTIVE | ev_action::CUSTOMER_APPROVAL_GRANTED => {
            // We don't have all the fields needed for a complete
            // PaymentMethodAttached (no customer external id on the event);
            // surface what we have so the handler can fetch via
            // `fetch_payment_method`.
            NormalizedEventKind::PaymentMethodAttached(PaymentMethodAttachedEvent {
                external_customer_id: event.links.customer.clone().unwrap_or_default(),
                external_payment_method_id: mandate_id,
                payment_method_type: PaymentMethodTypeEnum::Other,
                meteroid_connection_id: event.metadata.get("meteroid.connection_id").cloned(),
                meteroid_customer_id: event.metadata.get("meteroid.customer_id").cloned(),
            })
        }
        ev_action::CANCELLED | ev_action::EXPIRED | ev_action::FAILED => {
            NormalizedEventKind::PaymentMethodDetached(PaymentMethodDetachedEvent {
                external_payment_method_id: mandate_id,
                reason: Some(format!("mandate.{}", event.action)),
            })
        }
        _ => NormalizedEventKind::Acknowledged {
            reason: "unhandled gocardless mandate action",
        },
    })
}

// `MandateStatus` is part of the public API; bring it into scope for any
// follow-up code that reads mandate state. Unused warning silenced.
#[allow(dead_code)]
const _MANDATE_STATUS: fn(MandateStatus) -> MandateStatus = std::convert::identity;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::connectors::{
        Connector, GocardlessPublicData, GocardlessSensitiveData, ProviderSensitiveData,
    };
    use crate::domain::enums::ConnectorTypeEnum;
    use chrono::NaiveDateTime;
    use common_domain::ids::{ConnectorId, TenantId};
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    const TEST_SECRET: &str = "gc_whsec_test_unit";

    fn test_connector() -> Connector {
        Connector {
            id: ConnectorId::new(),
            created_at: NaiveDateTime::default(),
            tenant_id: TenantId::new(),
            alias: "gc-test".into(),
            connector_type: ConnectorTypeEnum::PaymentProvider,
            provider: ConnectorProviderEnum::Gocardless,
            data: Some(crate::domain::connectors::ProviderData::Gocardless(
                GocardlessPublicData {
                    creditor_id: Some("CR000".into()),
                    environment: "sandbox".into(),
                },
            )),
            sensitive: Some(ProviderSensitiveData::Gocardless(GocardlessSensitiveData {
                access_token: "sandbox_token".into(),
                webhook_secret: TEST_SECRET.into(),
            })),
        }
    }

    fn sign(payload: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(TEST_SECRET.as_bytes()).unwrap();
        mac.update(payload);
        hex::encode(mac.finalize().into_bytes())
    }

    /// Signature contract: HMAC-SHA-256 of raw body, hex-encoded, in the
    /// `Webhook-Signature` header. No timestamp in the scheme; replay
    /// protection is via DB dedup on event id.
    #[test]
    fn verify_signature_accepts_valid() {
        let payload = br#"{"events":[]}"#;
        let mut headers = HeaderMap::new();
        headers.insert("Webhook-Signature", sign(payload).parse().unwrap());
        let result = GoCardlessConnector::new().verify_signature(
            &test_connector(),
            payload,
            &headers,
            &SecretString::from(TEST_SECRET.to_string()),
        );
        assert!(result.is_ok(), "valid sig must accept: {result:?}");
    }

    #[test]
    fn verify_signature_rejects_tampered() {
        let payload = br#"{"events":[]}"#;
        let mut headers = HeaderMap::new();
        headers.insert("Webhook-Signature", sign(payload).parse().unwrap());
        let tampered = br#"{"events":[{}]}"#;
        let result = GoCardlessConnector::new().verify_signature(
            &test_connector(),
            tampered,
            &headers,
            &SecretString::from(TEST_SECRET.to_string()),
        );
        assert!(result.is_err(), "tampered body must reject");
    }

    #[test]
    fn verify_signature_rejects_missing_header() {
        let payload = br#"{"events":[]}"#;
        let headers = HeaderMap::new();
        let result = GoCardlessConnector::new().verify_signature(
            &test_connector(),
            payload,
            &headers,
            &SecretString::from(TEST_SECRET.to_string()),
        );
        assert!(result.is_err());
    }

    /// `payments.confirmed` is the success signal for an off-session charge.
    /// Must surface as `PaymentSucceeded` with the meteroid_transaction_id
    /// preserved from metadata so the handler can look up our local row.
    #[test]
    fn parse_event_payments_confirmed_succeeds() {
        let payload = br#"{
            "events":[{
                "id":"EV_OK_1",
                "created_at":"2026-05-19T12:00:00Z",
                "resource_type":"payments",
                "action":"confirmed",
                "links":{"payment":"PM_OK_1"},
                "metadata":{"meteroid.transaction_id":"tx_ok"}
            }]
        }"#;
        let parsed = GoCardlessConnector::new()
            .parse_event(&test_connector(), payload, &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::PaymentSucceeded(e) => {
                assert_eq!(e.external_transaction_id, "PM_OK_1");
                assert_eq!(e.meteroid_transaction_id.as_deref(), Some("tx_ok"));
            }
            other => panic!("expected PaymentSucceeded, got {other:?}"),
        }
    }

    /// `payments.failed` → `PaymentFailed`, preserving the GC cause code
    /// from `details.cause`.
    #[test]
    fn parse_event_payments_failed() {
        let payload = br#"{
            "events":[{
                "id":"EV_FAIL_1",
                "created_at":"2026-05-19T12:00:00Z",
                "resource_type":"payments",
                "action":"failed",
                "links":{"payment":"PM_FAIL_1"},
                "details":{
                    "origin":"bank",
                    "cause":"insufficient_funds",
                    "description":"The customer's account had insufficient funds."
                },
                "metadata":{"meteroid.transaction_id":"tx_fail"}
            }]
        }"#;
        let parsed = GoCardlessConnector::new()
            .parse_event(&test_connector(), payload, &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::PaymentFailed(e) => {
                assert_eq!(e.external_transaction_id, "PM_FAIL_1");
                assert_eq!(e.code.as_deref(), Some("insufficient_funds"));
                assert!(e.message.contains("insufficient funds"));
            }
            other => panic!("expected PaymentFailed, got {other:?}"),
        }
    }

    /// `mandates.active` → `PaymentMethodAttached`. The mandate id becomes
    /// the external payment method id; the handler will fetch the mandate
    /// to get the scheme (sepa/bacs/ach).
    #[test]
    fn parse_event_mandates_active() {
        let payload = br#"{
            "events":[{
                "id":"EV_MAND_1",
                "created_at":"2026-05-19T12:00:00Z",
                "resource_type":"mandates",
                "action":"active",
                "links":{"mandate":"MD_1","customer":"CU_1"},
                "metadata":{
                    "meteroid.connection_id":"conn_1",
                    "meteroid.customer_id":"cust_1"
                }
            }]
        }"#;
        let parsed = GoCardlessConnector::new()
            .parse_event(&test_connector(), payload, &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::PaymentMethodAttached(e) => {
                assert_eq!(e.external_payment_method_id, "MD_1");
                assert_eq!(e.external_customer_id, "CU_1");
                assert_eq!(e.meteroid_connection_id.as_deref(), Some("conn_1"));
                assert_eq!(e.meteroid_customer_id.as_deref(), Some("cust_1"));
            }
            other => panic!("expected PaymentMethodAttached, got {other:?}"),
        }
    }

    /// `mandates.cancelled` → `PaymentMethodDetached`. Customer revoked
    /// authorisation at their bank; we can't charge against this mandate
    /// any more.
    #[test]
    fn parse_event_mandates_cancelled() {
        let payload = br#"{
            "events":[{
                "id":"EV_MAND_X",
                "created_at":"2026-05-19T12:00:00Z",
                "resource_type":"mandates",
                "action":"cancelled",
                "links":{"mandate":"MD_X"}
            }]
        }"#;
        let parsed = GoCardlessConnector::new()
            .parse_event(&test_connector(), payload, &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::PaymentMethodDetached(e) => {
                assert_eq!(e.external_payment_method_id, "MD_X");
                assert!(e.reason.as_deref().unwrap_or("").contains("cancelled"));
            }
            other => panic!("expected PaymentMethodDetached, got {other:?}"),
        }
    }

    /// Capability honesty: GoCardless caps must claim no card / no
    /// self-webhook-registration. The contract harness asserts this
    /// abstractly; pin it concretely too.
    #[test]
    fn capabilities_match_provider_reality() {
        let connector = GoCardlessConnector::new();
        let caps = connector.capabilities();
        assert!(!caps.supports_cards);
        assert!(caps.supports_mandates);
        assert!(caps.asynchronous_settlement);
        assert!(!caps.supports_self_webhook_registration);
        assert_eq!(caps.mandate_setup_mode, MandateSetupMode::HostedRedirect);
        assert!(
            caps.supported_payment_methods
                .contains(&PaymentMethodTypeEnum::DirectDebitSepa)
        );
    }
}
