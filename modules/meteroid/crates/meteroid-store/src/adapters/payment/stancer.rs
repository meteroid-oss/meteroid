//! Stancer connector. Card provider with a hosted-page capture flow (no
//! client-side tokenization SDK): mandate setup creates a payment intent,
//! redirects to the hosted page, and completes server-side on the return
//! redirect. Plain add-payment-method is 0-amount `capture: false` (card save
//! only); hosted CHECKOUT / hosted INVOICE payment intents carry the REAL
//! amount with `capture: true`, captured in-flow on the hosted page — the id
//! surfaces as `PaymentMethodSnapshot.payment_request_payment` and there is NO
//! server-initiated charge for those paths.
//!
//! Stancer has **no webhook mechanism at all** (OpenAPI-verified): settlement
//! is asynchronous (`to_capture`/`capture_sent` → `captured`, nothing pushed)
//! and resolves through the reconciliation worker; mandate completion surfaces
//! through the return-URL redirect.

use super::connector::{
    ConnectorCapabilities, ConnectorIdentity, CustomerOps, HostedSetupCompletion, MandateOps,
    MandateSetupMode, PaymentOps, ReconcileOps, RefundOps, WebhookOps,
};
use super::error::{ConnectorError, HostedSetupPending};
use super::events::{NormalizedEventSubscription, NormalizedWebhookEvent};
use super::model::{
    ChargeAcknowledged, ChargeCancelled, ChargeFailure, ChargeOutcome, ChargeReceipt,
    ChargeRequest, CreateCustomerRequest, DeclineKind, ExternalCustomerRef,
    MandateSetupInstruction, MandateSetupRequest, PaymentMethodSnapshot, RefundOutcome,
    RefundRequest, RefundSnapshot, RegisteredWebhook, RemoteTransactionStatus,
};
use crate::domain::connectors::{Connector, ProviderSensitiveData};
use crate::domain::enums::ConnectorProviderEnum;
use crate::domain::{Customer, CustomerConnection, PaymentMethodTypeEnum};
use async_trait::async_trait;
use common_domain::ids::BaseId;
use error_stack::Report;
use http::HeaderMap;
use secrecy::SecretString;
use stancer_client::client::StancerClient;
use stancer_client::customers::CreateCustomer as StancerCreateCustomer;
use stancer_client::error::StancerError;
use stancer_client::payment_intents::{
    CreatePaymentIntent, PaymentIntentStatus, StancerPaymentIntent, StancerPaymentMethod,
    ThreeDsMode, UpdatePaymentIntent,
};
use stancer_client::payments::{CreatePayment, StancerPayment, StancerPaymentStatus};
use std::collections::HashMap;
use std::sync::OnceLock;

pub(super) const STANCER_CAPABILITIES: ConnectorCapabilities = ConnectorCapabilities {
    supports_cards: true,
    // The saved card token is a reusable off-session instrument (a "mandate").
    supports_mandates: true,
    // `refund()` returns Unsupported — refunds are not wired into the billing
    // layer for any provider yet.
    supports_refunds: false,
    supports_partial_refunds: false,
    // The hosted page runs the 3DS challenge (`threeds: required` on the
    // intent). Off-session charges omit `auth` → no 3DS (MIT posture).
    supports_3ds: true,
    supports_disputes: false,
    // No webhook mechanism at all (spec-verified) — nothing to register.
    supports_self_webhook_registration: false,
    // Charges land as `to_capture`/`capture_sent`; reconcile settles them.
    asynchronous_settlement: true,
    supported_payment_methods: &[PaymentMethodTypeEnum::Card],
    mandate_setup_mode: MandateSetupMode::HostedRedirect,
    // Moot (no webhooks, verify_signature always rejects) but the capability
    // contract requires a value > 0.
    webhook_replay_tolerance_secs: 3600,
    // The return redirect is the only push signal; the sweeper polls persisted
    // pending intents as the lost-return backstop.
    hosted_setup_completion: HostedSetupCompletion::PollingRequired,
};

/// Currencies Stancer accepts; anything else is rejected up-front.
const SUPPORTED_CURRENCIES: &[&str] = &[
    "eur", "aud", "cad", "chf", "dkk", "gbp", "nok", "pln", "sek", "usd",
];

/// One API base for test and live — the mode rides on the secret key prefix
/// (`stest_…` / `sprod_…`) — so a single client singleton serves all tenants.
#[derive(Debug, Clone, Copy)]
pub struct StancerConnector;

impl StancerConnector {
    pub fn new() -> Self {
        StancerConnector
    }

    fn client() -> &'static StancerClient {
        static CLIENT: OnceLock<StancerClient> = OnceLock::new();
        CLIENT.get_or_init(StancerClient::new)
    }
}

impl Default for StancerConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectorIdentity for StancerConnector {
    fn provider(&self) -> ConnectorProviderEnum {
        ConnectorProviderEnum::Stancer
    }
    fn capabilities(&self) -> &ConnectorCapabilities {
        &STANCER_CAPABILITIES
    }
}

#[async_trait]
impl CustomerOps for StancerConnector {
    /// Stancer customers have no metadata map; the unique `external_id`
    /// carries our customer id and doubles as the idempotency mechanism: a
    /// retried create conflicts on it and resolves by lookup instead of failing.
    async fn create_customer(
        &self,
        connector: &Connector,
        customer: &Customer,
        _request: CreateCustomerRequest,
    ) -> Result<ExternalCustomerRef, Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;
        let client = Self::client();

        let external_id = customer.id.as_base62();

        // Send only what Stancer requires (email or mobile) plus our
        // correlation id; mobile is a fallback when there is no email.
        let email = customer.billing_email.clone();
        let mobile = if email.is_none() {
            normalize_stancer_mobile(customer.phone.as_deref())
        } else {
            None
        };

        let result = client
            .create_customer(
                StancerCreateCustomer {
                    email,
                    mobile,
                    // The only correlation slot back to Meteroid (unique, ≤36).
                    external_id: Some(external_id.clone()),
                    ..Default::default()
                },
                &secret_key,
            )
            .await;

        match result {
            Ok(created) => Ok(ExternalCustomerRef {
                external_id: created.id,
                provider_request_id: None,
            }),
            // A 4xx can be a unique-external_id conflict from a retried
            // create: reuse the existing customer, else surface the original error.
            Err(StancerError::Stancer(req_err))
                if req_err.http_status >= 400 && req_err.http_status < 500 =>
            {
                match client
                    .list_customers_by_external_id(&external_id, &secret_key)
                    .await
                {
                    Ok(list) => {
                        if let Some(existing) = list
                            .customers
                            .into_iter()
                            .find(|c| c.external_id.as_deref() == Some(external_id.as_str()))
                        {
                            log::info!(
                                "Stancer customer for {external_id} already exists as {}; reusing",
                                existing.id
                            );
                            return Ok(ExternalCustomerRef {
                                external_id: existing.id,
                                provider_request_id: None,
                            });
                        }
                        Err(map_stancer_error(
                            StancerOp::Customer,
                            StancerError::Stancer(req_err),
                        ))
                    }
                    // Lookup failed: nothing to reuse — surface the create error.
                    Err(_) => Err(map_stancer_error(
                        StancerOp::Customer,
                        StancerError::Stancer(req_err),
                    )),
                }
            }
            Err(e) => Err(map_stancer_error(StancerOp::Customer, e)),
        }
    }
}

#[async_trait]
impl MandateOps for StancerConnector {
    /// Create the hosted-page payment intent — see [`setup_intent_posture`]
    /// for the amount/capture split. A follow-up PATCH bakes the intent's own
    /// id into `return_url` so the return handler can complete without a
    /// webhook. No idempotency field exists on intent creation; a duplicate
    /// intent from a retry is harmless (only ONE hosted page is ever shown).
    async fn initiate_mandate_setup(
        &self,
        connector: &Connector,
        connection: &CustomerConnection,
        request: MandateSetupRequest<'_>,
    ) -> Result<MandateSetupInstruction, Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;
        let client = Self::client();

        if !request
            .payment_methods
            .contains(&PaymentMethodTypeEnum::Card)
        {
            return Err(Report::new(ConnectorError::MandateSetup(
                "Stancer supports card payments only; no card method was requested".to_string(),
            )));
        }

        // `return_url` is required even for iframe embedding and is our only
        // completion signal — Stancer has no webhooks.
        let return_url = request.return_url.clone().ok_or_else(|| {
            Report::new(ConnectorError::MandateSetup(
                "Stancer mandate setup requires a return_url (hosted redirect is the only flow)"
                    .to_string(),
            ))
        })?;

        // Real customer/checkout/invoice currency — never a hardcoded
        // default; Stancer validates it even for the 0-amount save posture.
        let currency = request
            .checkout
            .as_ref()
            .map(|c| c.currency.as_str())
            .or(request
                .invoice_payment
                .as_ref()
                .map(|i| i.currency.as_str()))
            .or(request.currency.as_deref())
            .ok_or_else(|| {
                Report::new(ConnectorError::MandateSetup(
                    "no currency available for the Stancer setup intent".to_string(),
                ))
            })?;
        let currency = validate_currency(currency)
            .map_err(|msg| Report::new(ConnectorError::MandateSetup(msg)))?;

        let (amount, capture) =
            setup_intent_posture(request.checkout.as_ref(), request.invoice_payment.as_ref())
                .map_err(|msg| Report::new(ConnectorError::MandateSetup(msg)))?;

        let metadata = setup_intent_metadata(connector, connection, &request);

        let intent = client
            .create_payment_intent(
                CreatePaymentIntent {
                    amount,
                    currency,
                    customer: Some(connection.external_customer_id.clone()),
                    methods_allowed: vec![StancerPaymentMethod::Card],
                    capture,
                    return_url: Some(return_url.clone()),
                    metadata: Some(metadata),
                    threeds: Some(ThreeDsMode::Required),
                    order_id: None,
                },
                &secret_key,
            )
            .await
            .map_err(|e| map_stancer_error(StancerOp::Mandate, e))?;

        // Must succeed: without the intent id in the return URL the return
        // handler cannot finish the setup and there is no webhook fallback.
        let sep = if return_url.contains('?') { '&' } else { '?' };
        let final_return_url = format!("{return_url}{sep}intent={}", intent.id);
        let updated = client
            .update_payment_intent(
                &intent.id,
                UpdatePaymentIntent {
                    return_url: Some(final_return_url),
                    metadata: None,
                },
                &secret_key,
            )
            .await
            .map_err(|e| map_stancer_error(StancerOp::Mandate, e))?;

        Ok(MandateSetupInstruction::HostedRedirect {
            intent_id: updated.id,
            authorisation_url: updated.url,
            // The intent response carries no expiry.
            expires_at: None,
        })
    }

    async fn fetch_payment_method(
        &self,
        connector: &Connector,
        external_payment_method_id: &str,
        _external_customer_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;
        let card = Self::client()
            .get_card(external_payment_method_id, &secret_key)
            .await
            .map_err(|e| map_stancer_error(StancerOp::PaymentMethod, e))?;
        // Cards carry no metadata — meteroid_* fields stay None.
        Ok(snapshot_from_card(card, &HashMap::new(), None))
    }

    /// Server-side completion driven by the return-URL redirect. The intent's
    /// `.card` is the authoritative "done" signal (`status` is NOT keyed on —
    /// its terminal value for a 0-amount flow is unobserved). `.card` absent +
    /// dead intent → failed; absent otherwise → retryable ([`HostedSetupPending`]).
    async fn complete_mandate_setup(
        &self,
        connector: &Connector,
        intent_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;
        let client = Self::client();

        let intent: StancerPaymentIntent = client
            .get_payment_intent(intent_id, &secret_key)
            .await
            .map_err(|e| map_stancer_error(StancerOp::Mandate, e))?;

        let card_id = match &intent.card {
            Some(card_id) => card_id.clone(),
            None => {
                // Only a certainly-dead intent is terminal; anything else —
                // Unknown included — stays retryable.
                return if intent_is_terminally_dead(&intent.status) {
                    Err(Report::new(ConnectorError::MandateSetup(format!(
                        "Stancer payment intent {intent_id} ended {:?} with no saved card",
                        intent.status
                    ))))
                } else {
                    Err(Report::new(ConnectorError::MandateSetup(format!(
                        "Stancer payment intent {intent_id} has no card yet (status {:?})",
                        intent.status
                    )))
                    .attach_opaque(HostedSetupPending))
                };
            }
        };

        let card = client
            .get_card(&card_id, &secret_key)
            .await
            .map_err(|e| map_stancer_error(StancerOp::Mandate, e))?;

        let metadata = intent.metadata.clone().unwrap_or_default();
        // `.payment` is only populated when the intent carried an amount — the
        // in-flow-captured payment, which completion records instead of charging.
        Ok(snapshot_from_card(card, &metadata, intent.payment.clone()))
    }

    /// `DELETE /v2/payment_intents/{id}` — the spec's only cancellation route
    /// (the PATCH schema carries no `status`). `Ok(())` only when the intent
    /// is certainly dead (DELETE ok, 404, or a follow-up read shows
    /// `canceled`/`unpaid`). Any intent that still could — or did — capture
    /// money is `Err`, so callers fail closed instead of orphaning it.
    async fn cancel_mandate_setup(
        &self,
        connector: &Connector,
        intent_id: &str,
    ) -> Result<(), Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;
        let client = Self::client();

        match client.delete_payment_intent(intent_id, &secret_key).await {
            Ok(_) => Ok(()),
            Err(StancerError::Stancer(req_err)) if req_err.http_status == 404 => Ok(()),
            Err(StancerError::Stancer(req_err))
                if req_err.http_status >= 400 && req_err.http_status < 500 =>
            {
                // Not cancelable in its current state — read it to decide:
                // already dead (idempotent cancel) or still live/captured (surface).
                let intent = client
                    .get_payment_intent(intent_id, &secret_key)
                    .await
                    .map_err(|e| map_stancer_error(StancerOp::Mandate, e))?;
                if intent_is_terminally_dead(&intent.status) {
                    Ok(())
                } else {
                    // Unknown included: unproven-dead fails closed as
                    // not-cancelable, so the caller adopts instead of orphaning.
                    Err(Report::new(ConnectorError::MandateSetup(format!(
                        "Stancer payment intent {intent_id} is not cancelable \
                         (status {:?}, payment {:?}); refusing to orphan it",
                        intent.status, intent.payment
                    ))))
                }
            }
            Err(e) => Err(map_stancer_error(StancerOp::Mandate, e)),
        }
    }
}

#[async_trait]
impl PaymentOps for StancerConnector {
    /// `POST /v2/payments/` against the saved card. `auth` is omitted entirely
    /// — omission is what skips 3DS off-session (`auth: false` is rejected;
    /// live-verified). Never returns `RequiresAction`. `unique_id` (derived
    /// via [`stancer_unique_id`]) is Stancer's only dedup mechanism here, so a
    /// DB rollback + retry dedupes at the provider instead of charging twice.
    async fn charge_off_session(
        &self,
        connector: &Connector,
        request: ChargeRequest<'_>,
    ) -> Result<ChargeOutcome, Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;
        let client = Self::client();

        let currency = validate_currency(request.currency)
            .map_err(|msg| Report::new(ConnectorError::Charge(msg)))?;

        let unique_id = stancer_unique_id(request.idempotency_key.as_str());

        let result = client
            .create_payment(
                CreatePayment {
                    amount: request.amount_minor,
                    currency,
                    customer: Some(request.customer_external_id.to_string()),
                    card: Some(request.payment_method_external_id.to_string()),
                    description: None,
                    order_id: None,
                    unique_id: Some(unique_id.clone()),
                    capture: true,
                },
                &secret_key,
            )
            .await;

        match result {
            Ok(payment) => Ok(payment_to_outcome(payment, request.amount_minor)),
            // A 4xx can be the unicity check rejecting a retry of a charge
            // that went through: look it up by unique_id and report ITS state.
            Err(StancerError::Stancer(req_err))
                if req_err.http_status >= 400 && req_err.http_status < 500 =>
            {
                match client
                    .list_payments_by_unique_id(&unique_id, &secret_key)
                    .await
                {
                    Ok(list) => {
                        if let Some(existing) = list
                            .payments
                            .into_iter()
                            .find(|p| p.unique_id.as_deref() == Some(unique_id.as_str()))
                        {
                            log::info!(
                                "Stancer payment for unique_id {unique_id} already exists as {}; adopting",
                                existing.id
                            );
                            return Ok(payment_to_outcome(existing, request.amount_minor));
                        }
                        Err(map_stancer_error(
                            StancerOp::Charge,
                            StancerError::Stancer(req_err),
                        ))
                    }
                    Err(_) => Err(map_stancer_error(
                        StancerOp::Charge,
                        StancerError::Stancer(req_err),
                    )),
                }
            }
            Err(e) => Err(map_stancer_error(StancerOp::Charge, e)),
        }
    }
}

#[async_trait]
impl RefundOps for StancerConnector {
    async fn refund(
        &self,
        _connector: &Connector,
        _request: RefundRequest<'_>,
    ) -> Result<RefundOutcome, Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Stancer,
            capability: "refund",
        }))
    }

    /// `fetch_refund` exists to resolve amount-less refund *webhooks*; Stancer
    /// has no webhooks, so nothing can ever observe a refund this way.
    async fn fetch_refund(
        &self,
        _connector: &Connector,
        _external_refund_id: &str,
    ) -> Result<RefundSnapshot, Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Stancer,
            capability: "fetch_refund",
        }))
    }
}

#[async_trait]
impl ReconcileOps for StancerConnector {
    /// With no webhooks this is THE settlement path, not just a safety net:
    /// the reconcile worker polls every Pending transaction to resolution.
    async fn fetch_transaction_status(
        &self,
        connector: &Connector,
        external_transaction_id: &str,
    ) -> Result<RemoteTransactionStatus, Report<ConnectorError>> {
        let secret_key = extract_secret_key(connector)?;

        let result = Self::client()
            .get_payment(external_transaction_id, &secret_key)
            .await;
        match result {
            Ok(payment) => Ok(remote_status_from_payment(payment)),
            Err(StancerError::Stancer(req_err)) if req_err.http_status == 404 => {
                Ok(RemoteTransactionStatus::Unknown)
            }
            Err(e) => Err(map_stancer_error(StancerOp::Charge, e)),
        }
    }
}

#[async_trait]
impl WebhookOps for StancerConnector {
    async fn register_webhook(
        &self,
        _connector: &Connector,
        _url: &str,
        _subscriptions: &[NormalizedEventSubscription],
    ) -> Result<RegisteredWebhook, Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Stancer,
            capability: "webhook.register (Stancer has no webhook mechanism)",
        }))
    }

    async fn unregister_webhook(
        &self,
        _connector: &Connector,
        _endpoint_id: &str,
    ) -> Result<(), Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Stancer,
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
            provider: ConnectorProviderEnum::Stancer,
            capability: "webhook.sync",
        }))
    }

    /// No legitimate Stancer webhook exists, so any payload claiming to be
    /// one is rejected unconditionally.
    fn verify_signature(
        &self,
        _connector: &Connector,
        _payload: &[u8],
        _headers: &HeaderMap,
        _secret: &SecretString,
    ) -> Result<(), Report<ConnectorError>> {
        Err(Report::new(ConnectorError::SignatureVerification))
    }

    fn parse_event(
        &self,
        _connector: &Connector,
        _payload: &[u8],
        _headers: &HeaderMap,
    ) -> Result<Option<NormalizedWebhookEvent>, Report<ConnectorError>> {
        Err(Report::new(ConnectorError::PayloadDecode(
            "Stancer emits no webhook events".to_string(),
        )))
    }
}

// ── helpers ────────────────────────────────────────────────────────

/// Whether an intent is CERTAINLY dead (can never save a card or capture
/// again). Anything unproven — `Unknown` included — is NOT dead.
fn intent_is_terminally_dead(status: &PaymentIntentStatus) -> bool {
    matches!(
        status,
        PaymentIntentStatus::Canceled | PaymentIntentStatus::Unpaid
    )
}

/// `(amount, capture)` for the hosted setup intent. Checkout and invoice
/// contexts capture the REAL amount in-flow (mutually exclusive); only the
/// plain add-payment-method setup is a pure 0-amount card save.
fn setup_intent_posture(
    checkout: Option<&super::model::HostedCheckoutContext>,
    invoice_payment: Option<&super::model::HostedInvoicePaymentContext>,
) -> Result<(i64, bool), String> {
    let amount = match (checkout, invoice_payment) {
        (Some(_), Some(_)) => {
            return Err(
                "Stancer setup intent cannot carry both a checkout and an invoice payment"
                    .to_string(),
            );
        }
        (Some(ctx), None) => Some(ctx.amount_minor),
        (None, Some(ctx)) => Some(ctx.amount_minor),
        (None, None) => None,
    };
    match amount {
        Some(amount) if amount <= 0 => Err(format!(
            "Stancer hosted capture requires a positive amount, got {amount}"
        )),
        Some(amount) => Ok((amount, true)),
        None => Ok((0, false)),
    }
}

/// Correlation ids, plus exactly one of the invoice or checkout context with
/// the pre-created transaction the capture must be recorded onto.
fn setup_intent_metadata(
    connector: &Connector,
    connection: &CustomerConnection,
    request: &MandateSetupRequest<'_>,
) -> HashMap<String, String> {
    let mut metadata = HashMap::from([
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
    if let Some(invoice_payment) = &request.invoice_payment {
        metadata.insert(
            "meteroid.invoice_id".to_string(),
            invoice_payment.invoice_id.clone(),
        );
        metadata.insert(
            "meteroid.transaction_id".to_string(),
            invoice_payment.transaction_id.clone(),
        );
    } else if let Some(invoice_id) = &request.invoice_id {
        // Legacy 0-amount invoice setup: invoice only — completion falls back
        // to the fail-closed off-session charge.
        metadata.insert("meteroid.invoice_id".to_string(), invoice_id.clone());
    } else if let Some(checkout) = &request.checkout {
        metadata.insert(
            "meteroid.checkout_session_id".to_string(),
            checkout.checkout_session_id.clone(),
        );
        metadata.insert(
            "meteroid.transaction_id".to_string(),
            checkout.transaction_id.clone(),
        );
    }
    metadata
}

fn extract_secret_key(connector: &Connector) -> Result<SecretString, Report<ConnectorError>> {
    match &connector.sensitive {
        Some(ProviderSensitiveData::Stancer(d)) => Ok(SecretString::from(d.api_secret_key.clone())),
        Some(_) => Err(Report::new(ConnectorError::Configuration(
            "connector is not a stancer connector".into(),
        ))),
        None => Err(Report::new(ConnectorError::Configuration(
            "stancer connector has no api_secret_key".into(),
        ))),
    }
}

/// Caller's idempotency key → Stancer `unique_id` (≤36 chars): verbatim when
/// it fits, else a stable sha256 prefix — never raw truncation (collisions).
fn stancer_unique_id(key: &str) -> String {
    if key.len() <= 36 {
        key.to_string()
    } else {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(key.as_bytes());
        hex::encode(&digest[..16])
    }
}

/// Returns the lowercase code Stancer expects, or a clear caller-facing message.
fn validate_currency(currency: &str) -> Result<String, String> {
    let lower = currency.to_ascii_lowercase();
    if SUPPORTED_CURRENCIES.contains(&lower.as_str()) {
        Ok(lower)
    } else {
        Err(format!(
            "Stancer does not support currency {currency}; supported: {}",
            SUPPORTED_CURRENCIES.join(", ")
        ))
    }
}

/// Snapshot from a fetched card + intent metadata (cards carry no metadata).
fn snapshot_from_card(
    card: stancer_client::cards::StancerCard,
    metadata: &HashMap<String, String>,
    payment_request_payment: Option<String>,
) -> PaymentMethodSnapshot {
    PaymentMethodSnapshot {
        external_payment_method_id: card.id,
        payment_method_type: PaymentMethodTypeEnum::Card,
        account_number_hint: None,
        card_brand: card.brand,
        card_last4: Some(card.last4),
        card_exp_month: Some(card.exp_month as i32),
        card_exp_year: Some(card.exp_year as i32),
        meteroid_connection_id: metadata.get("meteroid.connection_id").cloned(),
        meteroid_customer_id: metadata.get("meteroid.customer_id").cloned(),
        meteroid_invoice_id: metadata.get("meteroid.invoice_id").cloned(),
        meteroid_checkout_session_id: metadata.get("meteroid.checkout_session_id").cloned(),
        meteroid_transaction_id: metadata.get("meteroid.transaction_id").cloned(),
        payment_request_payment,
    }
}

/// ISO-8583-style network response code (`"00"` = approved) → decline category.
fn decline_kind_for(response: Option<&str>) -> DeclineKind {
    match response {
        Some("51") => DeclineKind::InsufficientFunds,
        Some("54") => DeclineKind::CardExpired,
        Some("05") => DeclineKind::DoNotHonor,
        Some("59") => DeclineKind::Fraud,
        _ => DeclineKind::Other,
    }
}

/// Map the synchronous `POST /v2/payments/` response. Stancer captures in an
/// end-of-day batch, so `to_capture`/`capture_sent` (bank-authorized, funds
/// committed — Stancer shows these as "paid") settle like `captured`, not Pending.
fn payment_to_outcome(payment: StancerPayment, amount_minor: i64) -> ChargeOutcome {
    let id = payment.id;
    let response = payment.response.filter(|r| r != "00");
    match payment.status {
        Some(
            StancerPaymentStatus::Captured
            | StancerPaymentStatus::ToCapture
            | StancerPaymentStatus::CaptureSent,
        ) => ChargeOutcome::Succeeded(ChargeReceipt {
            external_id: id,
            // No separate settled figure exists on the create response.
            amount_received_minor: amount_minor,
            processed_at: chrono::Utc::now().naive_utc(),
            provider_request_id: None,
        }),
        // `authorized` can still expire/cancel; `Unknown` is unmodeled — never
        // fabricate success/failure, let reconcile re-poll to a modeled status.
        Some(
            StancerPaymentStatus::Authorize
            | StancerPaymentStatus::Authorized
            | StancerPaymentStatus::Capture
            | StancerPaymentStatus::Unknown,
        )
        | None => ChargeOutcome::Pending(ChargeAcknowledged {
            external_id: id,
            provider_request_id: None,
        }),
        Some(StancerPaymentStatus::Canceled) => ChargeOutcome::Cancelled(ChargeCancelled {
            external_id: Some(id),
            message: "Payment cancelled".to_string(),
            provider_request_id: None,
        }),
        // Funds were captured; the dispute lifecycle is out of scope in v1
        // (supports_disputes = false), so the money state is "succeeded".
        Some(StancerPaymentStatus::Disputed) => ChargeOutcome::Succeeded(ChargeReceipt {
            external_id: id,
            amount_received_minor: amount_minor,
            processed_at: chrono::Utc::now().naive_utc(),
            provider_request_id: None,
        }),
        Some(
            status @ (StancerPaymentStatus::Refused
            | StancerPaymentStatus::Failed
            | StancerPaymentStatus::Expired),
        ) => {
            let decline_kind = decline_kind_for(response.as_deref());
            ChargeOutcome::Failed(ChargeFailure {
                external_id: Some(id),
                code: response.clone(),
                message: format!(
                    "Stancer payment {status:?}{}",
                    response
                        .as_deref()
                        .map(|r| format!(" (network response {r})"))
                        .unwrap_or_default()
                ),
                retryable: false,
                decline_kind,
                provider_request_id: None,
            })
        }
    }
}

/// Same status table as [`payment_to_outcome`], onto the reconciliation shape.
fn remote_status_from_payment(payment: StancerPayment) -> RemoteTransactionStatus {
    let response = payment.response.filter(|r| r != "00");
    match payment.status {
        Some(
            StancerPaymentStatus::Captured
            | StancerPaymentStatus::ToCapture
            | StancerPaymentStatus::CaptureSent
            | StancerPaymentStatus::Disputed,
        ) => RemoteTransactionStatus::Succeeded {
            amount_received_minor: payment.amount,
            currency: payment.currency,
            processed_at: chrono::Utc::now().naive_utc(),
        },
        Some(
            StancerPaymentStatus::Authorize
            | StancerPaymentStatus::Authorized
            | StancerPaymentStatus::Capture
            | StancerPaymentStatus::Unknown,
        )
        | None => RemoteTransactionStatus::Pending,
        Some(StancerPaymentStatus::Canceled) => RemoteTransactionStatus::Cancelled,
        Some(
            status @ (StancerPaymentStatus::Refused
            | StancerPaymentStatus::Failed
            | StancerPaymentStatus::Expired),
        ) => RemoteTransactionStatus::Failed {
            code: response.clone(),
            message: format!("Stancer payment {status:?}"),
            decline_kind: decline_kind_for(response.as_deref()),
        },
    }
}

/// Which operation produced an error, so a 4xx maps to the right semantic
/// `ConnectorError` variant instead of everything reading as a failed charge.
#[derive(Clone, Copy, Debug)]
enum StancerOp {
    Customer,
    Mandate,
    PaymentMethod,
    Charge,
}

impl StancerOp {
    fn logical_error(self, msg: String) -> ConnectorError {
        match self {
            StancerOp::Customer => ConnectorError::CustomerOp(msg),
            StancerOp::Mandate => ConnectorError::MandateSetup(msg),
            // Reading a card is a read on the customer's setup.
            StancerOp::PaymentMethod => ConnectorError::CustomerOp(msg),
            StancerOp::Charge => ConnectorError::Charge(msg),
        }
    }
}

fn map_stancer_error(op: StancerOp, e: StancerError) -> Report<ConnectorError> {
    match e {
        StancerError::ClientError(msg) => Report::new(ConnectorError::Transport(msg)),
        StancerError::Stancer(req_err) if req_err.http_status >= 500 => Report::new(
            ConnectorError::Transport(format!("stancer 5xx ({}): {req_err}", req_err.http_status)),
        ),
        // Rejected credentials are a configuration problem, NEVER a logical
        // failure: `MandateSetup` here would read as terminal and let the
        // sweeper expire every awaiting checkout over a mere key rotation.
        StancerError::Stancer(req_err) if matches!(req_err.http_status, 401 | 403) => {
            Report::new(ConnectorError::Configuration(format!(
                "stancer rejected credentials ({}): {req_err}",
                req_err.http_status
            )))
        }
        // Throttling / request timeout: transient, retryable.
        StancerError::Stancer(req_err) if matches!(req_err.http_status, 408 | 429) => {
            Report::new(ConnectorError::Transport(format!(
                "stancer transient ({}): {req_err}",
                req_err.http_status
            )))
        }
        // Remaining 4xx: a genuine validation/state error on the resource
        // itself — not retryable.
        StancerError::Stancer(req_err) => Report::new(op.logical_error(format!(
            "stancer rejected ({}): {req_err}",
            req_err.http_status
        ))),
        StancerError::JSONSerialize(e) => {
            Report::new(ConnectorError::Configuration(format!("stancer: {e}")))
        }
    }
}

/// Stancer's `mobile` is capped at 16 chars and format-validated: strip
/// whitespace and drop it entirely if it still doesn't fit — never fail
/// customer creation over an optional contact field.
fn normalize_stancer_mobile(phone: Option<&str>) -> Option<String> {
    let cleaned: String = phone?.chars().filter(|c| !c.is_whitespace()).collect();
    (!cleaned.is_empty() && cleaned.chars().count() <= 16).then_some(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::connectors::{Connector, StancerPublicData, StancerSensitiveData};
    use crate::domain::enums::ConnectorTypeEnum;
    use chrono::NaiveDateTime;
    use common_domain::ids::{ConnectorId, TenantId};

    #[test]
    fn normalize_stancer_mobile_strips_spaces_and_bounds_length() {
        // Spaced number (17 chars) fits once whitespace is stripped (12).
        assert_eq!(
            normalize_stancer_mobile(Some("+00 0 00 00 00 00")),
            Some("+00000000000".to_string())
        );
        assert_eq!(
            normalize_stancer_mobile(Some("+00000000000")),
            Some("+00000000000".to_string())
        );
        // Still over 16 chars after stripping → dropped, not an error.
        assert_eq!(normalize_stancer_mobile(Some("00000000000000000")), None);
        // Absent / whitespace-only → dropped.
        assert_eq!(normalize_stancer_mobile(None), None);
        assert_eq!(normalize_stancer_mobile(Some("   ")), None);
    }

    fn test_connector() -> Connector {
        Connector {
            id: ConnectorId::new(),
            created_at: NaiveDateTime::default(),
            tenant_id: TenantId::new(),
            alias: "stancer-test".into(),
            connector_type: ConnectorTypeEnum::PaymentProvider,
            provider: ConnectorProviderEnum::Stancer,
            data: Some(crate::domain::connectors::ProviderData::Stancer(
                StancerPublicData::default(),
            )),
            sensitive: Some(ProviderSensitiveData::Stancer(StancerSensitiveData {
                api_secret_key: "stest_unit".into(),
            })),
        }
    }

    fn payment(status: Option<StancerPaymentStatus>, response: Option<&str>) -> StancerPayment {
        StancerPayment {
            id: "paym_test1".into(),
            amount: 4_200,
            currency: "eur".into(),
            status,
            response: response.map(str::to_string),
            card: None,
            order_id: None,
            unique_id: None,
        }
    }

    /// An unmodeled (`Unknown`) status is retryable / not-cancelable, never
    /// dead; only `canceled`/`unpaid` are provably dead.
    #[test]
    fn unknown_intent_status_is_not_terminally_dead() {
        assert!(!intent_is_terminally_dead(&PaymentIntentStatus::Unknown));
        for live in [
            PaymentIntentStatus::RequirePaymentMethod,
            PaymentIntentStatus::RequireAuthentication,
            PaymentIntentStatus::RequireAuthorization,
            PaymentIntentStatus::Authorized,
            PaymentIntentStatus::Processing,
            PaymentIntentStatus::Captured,
        ] {
            assert!(
                !intent_is_terminally_dead(&live),
                "{live:?} must not read as dead"
            );
        }
        assert!(intent_is_terminally_dead(&PaymentIntentStatus::Canceled));
        assert!(intent_is_terminally_dead(&PaymentIntentStatus::Unpaid));
    }

    #[test]
    fn capabilities_match_provider_reality() {
        let connector = StancerConnector::new();
        let caps = connector.capabilities();
        assert!(caps.supports_cards);
        assert!(caps.supports_mandates);
        assert!(!caps.supports_refunds, "refund() is Unsupported");
        assert!(!caps.supports_partial_refunds);
        assert!(caps.supports_3ds);
        assert!(!caps.supports_disputes);
        assert!(!caps.supports_self_webhook_registration);
        assert!(caps.asynchronous_settlement);
        assert_eq!(caps.mandate_setup_mode, MandateSetupMode::HostedRedirect);
        assert_eq!(
            caps.supported_payment_methods,
            &[PaymentMethodTypeEnum::Card]
        );
    }

    /// `disputed` means funds were captured, so it counts as a success.
    #[test]
    fn payment_to_outcome_success_states() {
        match payment_to_outcome(
            payment(Some(StancerPaymentStatus::Captured), Some("00")),
            4_200,
        ) {
            ChargeOutcome::Succeeded(r) => {
                assert_eq!(r.external_id, "paym_test1");
                assert_eq!(r.amount_received_minor, 4_200);
            }
            other => panic!("expected Succeeded, got {other:?}"),
        }
        assert!(matches!(
            payment_to_outcome(payment(Some(StancerPaymentStatus::Disputed), None), 4_200),
            ChargeOutcome::Succeeded(_)
        ));
        // Batch capture: to_capture/capture_sent are bank-authorized and committed
        // (Stancer shows them "paid"), so they settle immediately like captured.
        for status in [
            StancerPaymentStatus::ToCapture,
            StancerPaymentStatus::CaptureSent,
        ] {
            assert!(
                matches!(
                    payment_to_outcome(payment(Some(status.clone()), None), 4_200),
                    ChargeOutcome::Succeeded(_)
                ),
                "status {status:?} must be Succeeded"
            );
        }
    }

    /// Unmodeled statuses must never be fabricated into a success or failure.
    #[test]
    fn payment_to_outcome_pending_states() {
        for status in [
            Some(StancerPaymentStatus::Authorize),
            Some(StancerPaymentStatus::Authorized),
            Some(StancerPaymentStatus::Capture),
            Some(StancerPaymentStatus::Unknown),
            None,
        ] {
            assert!(
                matches!(
                    payment_to_outcome(payment(status.clone(), None), 100),
                    ChargeOutcome::Pending(_)
                ),
                "status {status:?} must be Pending"
            );
        }
    }

    /// Declines are terminal and carry the network code; `canceled` is distinct.
    #[test]
    fn payment_to_outcome_terminal_states() {
        match payment_to_outcome(
            payment(Some(StancerPaymentStatus::Refused), Some("51")),
            100,
        ) {
            ChargeOutcome::Failed(f) => {
                assert!(!f.retryable);
                assert_eq!(f.code.as_deref(), Some("51"));
                assert_eq!(f.decline_kind, DeclineKind::InsufficientFunds);
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(matches!(
            payment_to_outcome(payment(Some(StancerPaymentStatus::Expired), None), 100),
            ChargeOutcome::Failed(_)
        ));
        assert!(matches!(
            payment_to_outcome(payment(Some(StancerPaymentStatus::Failed), Some("05")), 100),
            ChargeOutcome::Failed(_)
        ));
        assert!(matches!(
            payment_to_outcome(payment(Some(StancerPaymentStatus::Canceled), None), 100),
            ChargeOutcome::Cancelled(_)
        ));
    }

    /// `"00"` means approved — it must never surface as a decline code.
    #[test]
    fn approved_response_code_is_not_a_decline_code() {
        match payment_to_outcome(
            payment(Some(StancerPaymentStatus::Refused), Some("00")),
            100,
        ) {
            ChargeOutcome::Failed(f) => assert_eq!(f.code, None),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn decline_kind_map() {
        assert_eq!(decline_kind_for(Some("51")), DeclineKind::InsufficientFunds);
        assert_eq!(decline_kind_for(Some("54")), DeclineKind::CardExpired);
        assert_eq!(decline_kind_for(Some("05")), DeclineKind::DoNotHonor);
        assert_eq!(decline_kind_for(Some("59")), DeclineKind::Fraud);
        assert_eq!(decline_kind_for(Some("91")), DeclineKind::Other);
        assert_eq!(decline_kind_for(None), DeclineKind::Other);
    }

    #[test]
    fn remote_status_table() {
        assert!(matches!(
            remote_status_from_payment(payment(Some(StancerPaymentStatus::Captured), None)),
            RemoteTransactionStatus::Succeeded {
                amount_received_minor: 4_200,
                ..
            }
        ));
        assert!(matches!(
            remote_status_from_payment(payment(Some(StancerPaymentStatus::Disputed), None)),
            RemoteTransactionStatus::Succeeded { .. }
        ));
        for settled in [
            StancerPaymentStatus::ToCapture,
            StancerPaymentStatus::CaptureSent,
        ] {
            assert!(
                matches!(
                    remote_status_from_payment(payment(Some(settled.clone()), None)),
                    RemoteTransactionStatus::Succeeded { .. }
                ),
                "status {settled:?} must reconcile as Succeeded"
            );
        }
        for pending in [
            Some(StancerPaymentStatus::Authorize),
            Some(StancerPaymentStatus::Capture),
            Some(StancerPaymentStatus::Unknown),
            None,
        ] {
            assert!(
                matches!(
                    remote_status_from_payment(payment(pending.clone(), None)),
                    RemoteTransactionStatus::Pending
                ),
                "status {pending:?} must reconcile as Pending"
            );
        }
        assert!(matches!(
            remote_status_from_payment(payment(Some(StancerPaymentStatus::Canceled), None)),
            RemoteTransactionStatus::Cancelled
        ));
        match remote_status_from_payment(payment(Some(StancerPaymentStatus::Refused), Some("54"))) {
            RemoteTransactionStatus::Failed {
                code, decline_kind, ..
            } => {
                assert_eq!(code.as_deref(), Some("54"));
                assert_eq!(decline_kind, DeclineKind::CardExpired);
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// The same stable key must always yield the same ≤36-char unique_id, so
    /// a retry after a DB rollback dedupes at Stancer instead of double-charging.
    #[test]
    fn unique_id_is_stable_and_bounded() {
        assert_eq!(stancer_unique_id("charge:abc123"), "charge:abc123");
        let exactly_36 = "x".repeat(36);
        assert_eq!(stancer_unique_id(&exactly_36), exactly_36);

        // A long key maps to a stable hashed token, never raw truncation.
        let long_key = "charge:stancer-charge:6nR9DFeSRvUsdBkyugTQvC:2XkD9chDvUnQ3PZg4dKMxK:0";
        assert!(long_key.len() > 36);
        let first = stancer_unique_id(long_key);
        let second = stancer_unique_id(long_key);
        assert_eq!(
            first, second,
            "same key must always yield the same unique_id"
        );
        assert!(
            first.len() <= 36,
            "unique_id must fit Stancer's 36-char cap"
        );
        assert!(!long_key.contains(&first), "must be hashed, not truncated");

        // Distinct long keys sharing a 36-char prefix must not collide.
        let sibling = "charge:stancer-charge:6nR9DFeSRvUsdBkyugTQvC:2XkD9chDvUnQ3PZg4dKMxK:1";
        assert_ne!(first, stancer_unique_id(sibling));
    }

    /// The same stable idempotency key derives the same provider unique_id
    /// even with different transaction ids — the rollback-retry contract.
    #[test]
    fn charge_requests_with_same_key_share_a_unique_id() {
        use super::super::model::IdempotencyKey;
        use common_domain::ids::PaymentTransactionId;

        let seed = "charge:stancer-charge:methodB62xxxxxxxxxxxxx:invoiceB62xxxxxxxxxxxx:0";
        let make = |transaction_id| ChargeRequest {
            transaction_id,
            customer_external_id: "cust_ext",
            payment_method_external_id: "card_ext",
            payment_method_type: PaymentMethodTypeEnum::Card,
            amount_minor: 4_200,
            currency: "EUR",
            idempotency_key: IdempotencyKey::new(seed),
            on_session: false,
        };
        let first = make(PaymentTransactionId::new());
        let second = make(PaymentTransactionId::new());
        assert_ne!(first.transaction_id, second.transaction_id);
        assert_eq!(
            stancer_unique_id(first.idempotency_key.as_str()),
            stancer_unique_id(second.idempotency_key.as_str()),
        );
    }

    /// 401/403 must map to `Configuration`, never `MandateSetup` (terminal
    /// `SetupFailed` would let the sweeper expire every awaiting checkout over
    /// a key rotation); 404/422 stay logical; 408/429/5xx stay `Transport`.
    #[test]
    fn auth_errors_are_configuration_not_terminal_setup_failure() {
        use stancer_client::error::RequestError;

        let err_with_status = |status: u16| {
            StancerError::Stancer(RequestError {
                http_status: status,
                detail: vec![],
            })
        };

        for status in [401u16, 403] {
            let report = map_stancer_error(StancerOp::Mandate, err_with_status(status));
            assert!(
                matches!(report.current_context(), ConnectorError::Configuration(_)),
                "{status} must map to Configuration, got {:?}",
                report.current_context()
            );
            // The completion loop keys terminal SetupFailed on MandateSetup —
            // an auth error must never satisfy that predicate.
            assert!(
                !matches!(report.current_context(), ConnectorError::MandateSetup(_)),
                "{status} must NOT read as a terminal mandate-setup failure"
            );
        }

        for status in [408u16, 429] {
            let report = map_stancer_error(StancerOp::Mandate, err_with_status(status));
            assert!(
                matches!(report.current_context(), ConnectorError::Transport(_)),
                "{status} must map to retryable Transport"
            );
        }

        // Genuine per-intent state errors keep their logical mapping.
        for status in [404u16, 422] {
            let report = map_stancer_error(StancerOp::Mandate, err_with_status(status));
            assert!(
                matches!(report.current_context(), ConnectorError::MandateSetup(_)),
                "{status} must stay a logical mandate error"
            );
        }
        assert!(matches!(
            map_stancer_error(StancerOp::Charge, err_with_status(500)).current_context(),
            ConnectorError::Transport(_)
        ));
    }

    #[test]
    fn currency_validation() {
        assert_eq!(validate_currency("EUR").as_deref(), Ok("eur"));
        assert_eq!(validate_currency("usd").as_deref(), Ok("usd"));
        assert_eq!(validate_currency("Gbp").as_deref(), Ok("gbp"));
        assert!(validate_currency("JPY").is_err());
        assert!(validate_currency("BRL").is_err());
    }

    /// Intent metadata must round-trip into the snapshot for the ownership
    /// check and post-completion settlement.
    #[test]
    fn snapshot_recovers_intent_metadata() {
        let card = stancer_client::cards::StancerCard {
            id: "card_abc".into(),
            customer: Some("cust_1".into()),
            brand: Some("visa".into()),
            country: Some("FR".into()),
            exp_month: 12,
            exp_year: 2030,
            last4: "4242".into(),
            deleted: false,
        };
        let metadata = HashMap::from([
            ("meteroid.connection_id".to_string(), "conn_x".to_string()),
            ("meteroid.customer_id".to_string(), "cust_x".to_string()),
            ("meteroid.invoice_id".to_string(), "inv_x".to_string()),
            ("meteroid.transaction_id".to_string(), "tx_x".to_string()),
        ]);
        let snapshot = snapshot_from_card(card, &metadata, None);
        assert_eq!(snapshot.external_payment_method_id, "card_abc");
        assert_eq!(snapshot.payment_method_type, PaymentMethodTypeEnum::Card);
        assert_eq!(snapshot.card_brand.as_deref(), Some("visa"));
        assert_eq!(snapshot.card_last4.as_deref(), Some("4242"));
        assert_eq!(snapshot.card_exp_month, Some(12));
        assert_eq!(snapshot.card_exp_year, Some(2030));
        assert_eq!(snapshot.meteroid_connection_id.as_deref(), Some("conn_x"));
        assert_eq!(snapshot.meteroid_customer_id.as_deref(), Some("cust_x"));
        assert_eq!(snapshot.meteroid_invoice_id.as_deref(), Some("inv_x"));
        assert_eq!(snapshot.meteroid_transaction_id.as_deref(), Some("tx_x"));
        assert!(snapshot.meteroid_checkout_session_id.is_none());
    }

    /// No signature can ever verify, no payload can ever parse — never panic.
    #[test]
    fn webhooks_always_reject() {
        let connector = test_connector();
        let stancer = StancerConnector::new();
        let result = stancer.verify_signature(
            &connector,
            b"{}",
            &HeaderMap::new(),
            &SecretString::from("any".to_string()),
        );
        assert!(matches!(
            result.as_ref().err().map(|r| r.current_context()),
            Some(ConnectorError::SignatureVerification)
        ));

        let parsed = stancer.parse_event(&connector, b"{}", &HeaderMap::new());
        assert!(matches!(
            parsed.as_ref().err().map(|r| r.current_context()),
            Some(ConnectorError::PayloadDecode(_))
        ));
    }

    #[tokio::test]
    async fn refund_ops_are_unsupported() {
        let connector = test_connector();
        let stancer = StancerConnector::new();
        let refund = stancer
            .refund(
                &connector,
                RefundRequest {
                    external_transaction_id: "paym_x",
                    amount_minor: 100,
                    currency: "EUR",
                    reason: None,
                    idempotency_key: super::super::model::IdempotencyKey::new("k"),
                },
            )
            .await;
        assert!(matches!(
            refund.as_ref().err().map(|r| r.current_context()),
            Some(ConnectorError::Unsupported { .. })
        ));

        let fetched = stancer.fetch_refund(&connector, "refund_x").await;
        assert!(matches!(
            fetched.as_ref().err().map(|r| r.current_context()),
            Some(ConnectorError::Unsupported { .. })
        ));
    }

    /// Checkout/invoice intents carry the REAL amount with `capture: true`;
    /// only the plain add-payment-method setup stays a 0-amount card save.
    #[test]
    fn setup_intent_posture_captures_real_amount() {
        use super::super::model::{HostedCheckoutContext, HostedInvoicePaymentContext};

        let ctx = HostedCheckoutContext {
            tenant_id: "tenantB62".into(),
            checkout_session_id: "sessB62".into(),
            transaction_id: "txB62".into(),
            amount_minor: 12_345,
            currency: "EUR".into(),
        };
        assert_eq!(setup_intent_posture(Some(&ctx), None), Ok((12_345, true)));

        // Never a 0-amount save for an invoice payment (that would show
        // "0,00 €" and skip the charge).
        let invoice_ctx = HostedInvoicePaymentContext {
            invoice_id: "invB62".into(),
            transaction_id: "txB62".into(),
            amount_minor: 6_789,
            currency: "EUR".into(),
        };
        assert_eq!(
            setup_intent_posture(None, Some(&invoice_ctx)),
            Ok((6_789, true))
        );

        assert_eq!(setup_intent_posture(None, None), Ok((0, false)));

        // Never create a capturing intent for a non-positive amount.
        let zero = HostedCheckoutContext {
            amount_minor: 0,
            ..ctx.clone()
        };
        assert!(setup_intent_posture(Some(&zero), None).is_err());
        let negative = HostedCheckoutContext {
            amount_minor: -100,
            ..ctx.clone()
        };
        assert!(setup_intent_posture(Some(&negative), None).is_err());
        let zero_invoice = HostedInvoicePaymentContext {
            amount_minor: 0,
            ..invoice_ctx.clone()
        };
        assert!(setup_intent_posture(None, Some(&zero_invoice)).is_err());

        // Checkout and invoice contexts are mutually exclusive.
        assert!(setup_intent_posture(Some(&ctx), Some(&invoice_ctx)).is_err());
    }

    /// Checkout metadata names the session AND transaction; in-flow invoice
    /// the invoice AND transaction; legacy invoice the invoice only.
    #[test]
    fn setup_intent_metadata_carries_the_right_ids() {
        use super::super::model::{
            HostedCheckoutContext, HostedInvoicePaymentContext, IdempotencyKey,
        };
        use crate::domain::CustomerConnection;
        use common_domain::ids::{CustomerConnectionId, CustomerId};

        let connector = test_connector();
        let connection = CustomerConnection {
            id: CustomerConnectionId::new(),
            customer_id: CustomerId::new(),
            connector_id: connector.id,
            external_customer_id: "cust_ext".into(),
            supported_payment_types: Some(vec![PaymentMethodTypeEnum::Card]),
        };
        let base_request =
            |invoice_id: Option<String>,
             checkout: Option<HostedCheckoutContext>,
             invoice_payment: Option<HostedInvoicePaymentContext>| {
                MandateSetupRequest {
                    payment_methods: &[PaymentMethodTypeEnum::Card],
                    idempotency_key: IdempotencyKey::new("k"),
                    return_url: Some("https://api.example.invalid/return".into()),
                    invoice_id,
                    checkout,
                    invoice_payment,
                    currency: Some("EUR".into()),
                }
            };

        let checkout_request = base_request(
            None,
            Some(HostedCheckoutContext {
                tenant_id: connector.tenant_id.as_base62(),
                checkout_session_id: "sessB62".into(),
                transaction_id: "txB62".into(),
                amount_minor: 4_200,
                currency: "EUR".into(),
            }),
            None,
        );
        let metadata = setup_intent_metadata(&connector, &connection, &checkout_request);
        assert_eq!(
            metadata
                .get("meteroid.checkout_session_id")
                .map(String::as_str),
            Some("sessB62")
        );
        assert_eq!(
            metadata.get("meteroid.transaction_id").map(String::as_str),
            Some("txB62")
        );
        assert_eq!(
            metadata.get("meteroid.connection_id"),
            Some(&connection.id.as_base62())
        );
        assert_eq!(
            metadata.get("meteroid.customer_id"),
            Some(&connection.customer_id.as_base62())
        );
        assert!(!metadata.contains_key("meteroid.invoice_id"));

        let in_flow_invoice_request = base_request(
            Some("invB62".into()),
            None,
            Some(HostedInvoicePaymentContext {
                invoice_id: "invB62".into(),
                transaction_id: "txB62".into(),
                amount_minor: 6_789,
                currency: "EUR".into(),
            }),
        );
        let metadata = setup_intent_metadata(&connector, &connection, &in_flow_invoice_request);
        assert_eq!(
            metadata.get("meteroid.invoice_id").map(String::as_str),
            Some("invB62")
        );
        assert_eq!(
            metadata.get("meteroid.transaction_id").map(String::as_str),
            Some("txB62")
        );
        assert!(!metadata.contains_key("meteroid.checkout_session_id"));

        // Legacy setup carries no transaction id — completion keys the
        // off-session fallback on its absence.
        let invoice_request = base_request(Some("invB62".into()), None, None);
        let metadata = setup_intent_metadata(&connector, &connection, &invoice_request);
        assert_eq!(
            metadata.get("meteroid.invoice_id").map(String::as_str),
            Some("invB62")
        );
        assert!(!metadata.contains_key("meteroid.checkout_session_id"));
        assert!(!metadata.contains_key("meteroid.transaction_id"));
    }

    /// A non-card method request must be rejected before any provider call.
    #[tokio::test]
    async fn initiate_setup_requires_card_method() {
        use crate::domain::CustomerConnection;
        use common_domain::ids::{CustomerConnectionId, CustomerId};

        let connector = test_connector();
        let connection = CustomerConnection {
            id: CustomerConnectionId::new(),
            customer_id: CustomerId::new(),
            connector_id: connector.id,
            external_customer_id: "cust_ext".into(),
            supported_payment_types: Some(vec![PaymentMethodTypeEnum::Card]),
        };
        let result = StancerConnector::new()
            .initiate_mandate_setup(
                &connector,
                &connection,
                MandateSetupRequest {
                    payment_methods: &[PaymentMethodTypeEnum::DirectDebitSepa],
                    idempotency_key: super::super::model::IdempotencyKey::new("k"),
                    return_url: Some("https://api.example.invalid/return".into()),
                    invoice_id: None,
                    checkout: None,
                    invoice_payment: None,
                    currency: Some("EUR".into()),
                },
            )
            .await;
        assert!(matches!(
            result.as_ref().err().map(|r| r.current_context()),
            Some(ConnectorError::MandateSetup(_))
        ));
    }
}
