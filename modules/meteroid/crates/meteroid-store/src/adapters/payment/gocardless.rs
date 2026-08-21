//! GoCardless connector. Mandate setup is hosted-redirect (no embedded SDK):
//! create a Billing Request, mint a Flow, redirect to `authorisation_url`, then
//! complete on return. Settlement is asynchronous — `POST /payments` returns
//! `pending_submission` and final state arrives 3–5 business days later via
//! webhook. Webhook endpoints are dashboard-managed; self-registration is
//! unsupported.

use super::connector::{
    ConnectorCapabilities, ConnectorIdentity, CustomerOps, MandateOps, MandateSetupMode,
    PaymentOps, ReconcileOps, RefundOps, WebhookOps,
};
use super::error::ConnectorError;
use super::events::{
    NormalizedEventKind, NormalizedEventSubscription, NormalizedWebhookEvent, PaymentFailedEvent,
    PaymentMethodDetachedEvent, PaymentReinstatedEvent, PaymentReversedEvent,
    PaymentSucceededEvent,
};
use super::model::{
    ChargeAcknowledged, ChargeCancelled, ChargeFailure, ChargeOutcome, ChargeReceipt,
    ChargeRequest, CreateCustomerRequest, DeclineKind, ExternalCustomerRef,
    MandateSetupInstruction, MandateSetupRequest, PaymentMethodSnapshot, RefundOutcome,
    RefundRequest, RefundSnapshot, RegisteredWebhook, RemoteTransactionStatus,
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
    CreateBillingRequestFlow, MandateRequest, PaymentRequest,
};
use gocardless_client::client::GoCardlessClient;
use gocardless_client::customer_bank_accounts::CustomerBankAccountApi;
use gocardless_client::customers::{CreateCustomer, CustomerApi};
use gocardless_client::error::GoCardlessError;
use gocardless_client::mandates::MandateApi;
use gocardless_client::payments::{CreatePayment, CreatePaymentLinks, PaymentApi, PaymentStatus};
use gocardless_client::refunds::RefundApi;
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
    // `refund()` is not implemented yet (returns Unsupported); advertising the
    // capability would be dishonest. Flip both back on once refunds land.
    supports_refunds: false,
    supports_partial_refunds: false,
    supports_3ds: false,
    supports_disputes: true,
    // No public API to manage webhook endpoints; merchants configure them in
    // the dashboard and paste the signing secret into our connect form.
    supports_self_webhook_registration: false,
    asynchronous_settlement: true,
    supported_payment_methods: &[
        PaymentMethodTypeEnum::DirectDebitSepa,
        PaymentMethodTypeEnum::DirectDebitAch,
        PaymentMethodTypeEnum::DirectDebitBacs,
    ],
    mandate_setup_mode: MandateSetupMode::HostedRedirect,
    // No timestamp in the signature header, so this tolerance is not enforced
    // at the header level; replay protection is per-event (provider_config_id,
    // provider_event_id) DB dedup — the router splits each batched GoCardless
    // event into its own audit row keyed by the event's `EV...` id. Value
    // surfaced for capability honesty.
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
        // Default to sandbox when the data blob is missing or malformed: a
        // misconfig must never silently route to live and charge real money.
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

        // All-or-nothing, gated on country_code: GoCardless rejects a partial
        // address (lines with no valid country). With no country we send no
        // address block — the hosted BRF collects it on the redirect page.
        let (addr1, addr2, city, region, postal_code, country_code) = customer
            .billing_address
            .as_ref()
            .filter(|a| a.country.is_some())
            .map_or((None, None, None, None, None, None), |a| {
                (
                    a.line1.clone(),
                    a.line2.clone(),
                    a.city.clone(),
                    a.state.clone(),
                    a.zip_code.clone(),
                    a.country.as_ref().map(|c| c.code.clone()),
                )
            });

        let result = client
            .create_customer(
                CreateCustomer {
                    email: customer.billing_email.clone(),
                    given_name: Some(given_name),
                    family_name: Some(family_name),
                    company_name: Some(customer.name.clone()),
                    language: None,
                    // Omitted: GoCardless validates phone_number to E.164 and
                    // rejects the whole customer create on a bad value. Our phone
                    // is free-form, and phone is optional at GC — so skip it.
                    phone_number: None,
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
            .map_err(|e| map_gc_error(GcOp::Customer, e))?;

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

        // GoCardless infers the scheme from currency; passing scheme is optional.
        let (currency, scheme) = request
            .payment_methods
            .iter()
            .find_map(method_to_currency_scheme)
            .ok_or_else(|| {
                Report::new(ConnectorError::MandateSetup(
                    "no GoCardless-compatible payment method requested".to_string(),
                ))
            })?;

        // GoCardless caps metadata at 3 properties. Tenant is derived from the
        // connector server-side and is never read back from metadata, so we omit
        // it here to leave room for the invoice id below — keeping us within the
        // cap on both the mandate_request and the billing_request metadata.
        let mut metadata = HashMap::from([
            (
                "meteroid.customer_id".to_string(),
                connection.customer_id.as_base62(),
            ),
            (
                "meteroid.connection_id".to_string(),
                connection.id.as_base62(),
            ),
        ]);
        // 3rd/final metadata property (the cap is 3): either the invoice this
        // mandate pays (invoice-payment page — `billing_requests.fulfilled`
        // recovers it and charges the invoice) OR the checkout session this
        // hosted checkout completes (recovered to activate the subscription
        // in-flight). Mutually exclusive.
        if let Some(invoice_id) = &request.invoice_id {
            metadata.insert("meteroid.invoice_id".to_string(), invoice_id.clone());
        } else if let Some(checkout) = &request.checkout {
            metadata.insert(
                "meteroid.checkout_session_id".to_string(),
                checkout.checkout_session_id.clone(),
            );
        }

        // GoCardless collects the payment in the mandate scheme's currency; a
        // checkout whose currency differs (e.g. USD on a SEPA connection) must be
        // rejected rather than silently collected in EUR. Fail before creating
        // any Billing Request.
        if let Some(checkout) = request.checkout.as_ref()
            && !checkout.currency.eq_ignore_ascii_case(&currency)
        {
            return Err(Report::new(ConnectorError::MandateSetup(format!(
                "hosted checkout currency {} does not match the GoCardless mandate scheme currency {}",
                checkout.currency, currency
            ))));
        }

        // Hosted CHECKOUT: collect the first payment in the SAME hosted flow as
        // the mandate by attaching a `payment_request`. GoCardless creates the
        // payment on fulfillment (surfaced as `links.payment_request_payment`),
        // so there is no separate off-session charge. Its own metadata slot (a
        // distinct 3-property cap) carries our transaction id so the resulting
        // `payments.*` webhooks resolve straight to the local checkout tx.
        let payment_request = request.checkout.as_ref().map(|checkout| PaymentRequest {
            amount: checkout.amount_minor,
            currency: checkout.currency.clone(),
            description: Some(format!(
                "First payment for Meteroid customer {}",
                connection.customer_id.as_base62()
            )),
            metadata: Some(HashMap::from([
                ("meteroid.tenant_id".to_string(), checkout.tenant_id.clone()),
                (
                    "meteroid.transaction_id".to_string(),
                    checkout.transaction_id.clone(),
                ),
            ])),
        });

        let creditor_id = match &connector.data {
            Some(ProviderData::Gocardless(d)) => d.creditor_id.clone(),
            _ => None,
        };

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
                    payment_request,
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
            .map_err(|e| map_gc_error(GcOp::Mandate, e))?;

        // GoCardless redirects the payer to `redirect_uri` verbatim on completion
        // (which we observe via the `billing_requests.fulfilled` webhook, not on
        // this redirect) and to `exit_uri` on abandon — the exit marker lets the
        // return handler treat it as a graceful cancel.
        let redirect_uri = request.return_url.clone();
        let exit_uri = request.return_url.as_ref().map(|url| {
            let sep = if url.contains('?') { '&' } else { '?' };
            format!("{url}{sep}error=flow_abandoned")
        });

        let flow = client
            .create_billing_request_flow(
                CreateBillingRequestFlow {
                    redirect_uri,
                    exit_uri,
                    lock_currency: Some(true),
                    // No `lock_amount` here: GoCardless rejects it on a Billing
                    // Request Flow ("not a permitted key"); the payment_request
                    // amount is fixed by the Billing Request itself.
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
            .map_err(|e| map_gc_error(GcOp::Mandate, e))?;

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
            .map_err(|e| map_gc_error(GcOp::PaymentMethod, e))?;
        Ok(snapshot_from_mandate(
            mandate.id,
            mandate.scheme,
            &mandate.metadata,
            None,
        ))
    }

    /// Reads a fulfilled Billing Request and builds a snapshot of the resulting
    /// mandate. Driven by the `billing_requests.fulfilled` webhook (the hosted
    /// flow auto-fulfils asynchronously, so we OBSERVE completion via GET rather
    /// than POST `complete`, which would race fulfillment). The BR carries the
    /// `meteroid.*` metadata we set at creation — including `invoice_id` — so we
    /// recover our ids from the BR directly, with no dependency on GoCardless
    /// propagating mandate_request metadata onto the mandate.
    async fn complete_mandate_setup(
        &self,
        connector: &Connector,
        intent_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        let token = extract_access_token(connector)?;
        let client = Self::client_for(connector);

        let br = client
            .get_billing_request(intent_id, &token)
            .await
            .map_err(|e| map_gc_error(GcOp::Mandate, e))?;

        // The created mandate is linked as `mandate_request_mandate` on a
        // fulfilled BR (a billing request has no plain `mandate` link).
        let mandate_id = br.links.mandate_request_mandate.ok_or_else(|| {
            Report::new(ConnectorError::MandateSetup(format!(
                "GoCardless BR {} has no mandate yet (not fulfilled?)",
                intent_id
            )))
        })?;

        // Fetch the mandate to learn the scheme (sepa/bacs/ach), needed to tag
        // the payment method for downstream DD-type resolution.
        let mandate = client
            .get_mandate(&mandate_id, &token)
            .await
            .map_err(|e| map_gc_error(GcOp::Mandate, e))?;

        // Recover our ids from the BR's metadata (the authoritative record we
        // wrote at creation), overriding whatever the mandate happens to carry.
        let mut metadata = mandate.metadata.clone();
        for key in [
            "meteroid.connection_id",
            "meteroid.customer_id",
            "meteroid.invoice_id",
            "meteroid.checkout_session_id",
        ] {
            if let Some(value) = br.metadata.get(key) {
                metadata.insert(key.to_string(), value.clone());
            }
        }

        // For a combined mandate + first-payment checkout BR, GoCardless created
        // the first payment from our `payment_request`; carry its id so the
        // checkout transaction records it as `provider_transaction_id` and the
        // later `payments.*` webhooks settle it. Absent on mandate-only BRs.
        let payment_request_payment = br.links.payment_request_payment.clone();

        // Best-effort display hint: GoCardless returns the last digits of the
        // bank account (never the full number). A failure here must not fail
        // mandate setup — the account just shows without a "••••NN" preview.
        let bank_account_id = mandate.links.customer_bank_account.clone();
        let mut snapshot = snapshot_from_mandate(
            mandate.id,
            mandate.scheme,
            &metadata,
            payment_request_payment,
        );
        if let Some(bank_account_id) = bank_account_id {
            match client
                .get_customer_bank_account(&bank_account_id, &token)
                .await
            {
                Ok(ba) => snapshot.account_number_hint = ba.account_number_ending,
                Err(e) => log::debug!(
                    "could not fetch GoCardless bank account {bank_account_id} for display hint: {e}"
                ),
            }
        }

        Ok(snapshot)
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

        // DD schemes are single-currency (SEPA→EUR, BACS→GBP, ACH→USD); a
        // mismatched currency is rejected by GoCardless. Fail clearly here.
        if let Some((scheme_currency, _)) = method_to_currency_scheme(&request.payment_method_type)
            && !request.currency.eq_ignore_ascii_case(&scheme_currency)
        {
            return Err(Report::new(ConnectorError::Charge(format!(
                "GoCardless {:?} mandate is {}-only; cannot charge {}",
                request.payment_method_type, scheme_currency, request.currency
            ))));
        }

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
            Ok(payment) => Ok(payment_to_outcome(
                payment.id,
                payment.status,
                request.amount_minor,
            )),
            Err(e) => Err(map_gc_error(GcOp::Charge, e)),
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
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Gocardless,
            capability: "refund",
        }))
    }

    /// Resolve a webhook-observed refund (GoCardless refund events carry no
    /// amounts): read the refund for its parent payment, then the payment for
    /// its cumulative refunded total — the idempotent figure the reversal path
    /// reconciles against.
    async fn fetch_refund(
        &self,
        connector: &Connector,
        external_refund_id: &str,
    ) -> Result<RefundSnapshot, Report<ConnectorError>> {
        let token = extract_access_token(connector)?;
        let client = Self::client_for(connector);

        let refund = client
            .get_refund(external_refund_id, &token)
            .await
            .map_err(|e| map_gc_error(GcOp::Refund, e))?;
        let payment_id = refund.links.payment.clone().ok_or_else(|| {
            Report::new(ConnectorError::Refund(format!(
                "gocardless refund {external_refund_id} has no parent payment link"
            )))
        })?;
        let payment = client
            .get_payment(&payment_id, &token)
            .await
            .map_err(|e| map_gc_error(GcOp::Refund, e))?;

        Ok(RefundSnapshot {
            external_transaction_id: payment_id,
            cumulative_refunded_minor: payment.amount_refunded,
            currency: payment.currency,
        })
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
            Err(e) => Err(map_gc_error(GcOp::Charge, e)),
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
        connector: &Connector,
        payload: &[u8],
        headers: &HeaderMap,
    ) -> Result<Option<NormalizedWebhookEvent>, Report<ConnectorError>> {
        Ok(self
            .parse_events(connector, payload, headers)?
            .into_iter()
            .next())
    }

    /// GoCardless batches several events per POST; return all (dropping any
    /// loses it permanently once we ACK 200).
    fn parse_events(
        &self,
        _connector: &Connector,
        payload: &[u8],
        _headers: &HeaderMap,
    ) -> Result<Vec<NormalizedWebhookEvent>, Report<ConnectorError>> {
        let envelope: EventEnvelope = GoCardlessWebhook::parse_envelope(payload).map_err(|e| {
            Report::new(ConnectorError::PayloadDecode(format!(
                "failed to decode gocardless event envelope: {e}"
            )))
        })?;

        Ok(envelope
            .events
            .into_iter()
            .filter_map(normalize_event)
            .collect())
    }
}

// ── helpers ────────────────────────────────────────────────────────

fn extract_access_token(connector: &Connector) -> Result<SecretString, Report<ConnectorError>> {
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

/// Best-effort split into given/family; GoCardless requires both on customers
/// in many countries.
fn split_name(name: &str) -> (String, String) {
    let trimmed = name.trim();
    if let Some((first, rest)) = trimmed.split_once(' ') {
        (first.to_string(), rest.trim().to_string())
    } else {
        (trimmed.to_string(), trimmed.to_string())
    }
}

/// Maps a payment method to (primary currency, scheme); `None` for unsupported
/// methods (cards). Currency can be overridden per BRF.
fn method_to_currency_scheme(method: &PaymentMethodTypeEnum) -> Option<(String, &'static str)> {
    match method {
        PaymentMethodTypeEnum::DirectDebitSepa => Some(("EUR".into(), "sepa_core")),
        PaymentMethodTypeEnum::DirectDebitBacs => Some(("GBP".into(), "bacs")),
        PaymentMethodTypeEnum::DirectDebitAch => Some(("USD".into(), "ach")),
        _ => None,
    }
}

/// Surfaces our `meteroid.*` ids from the mandate metadata (propagated from the
/// BR's `mandate_request.metadata`), since mandate webhook events carry none.
fn snapshot_from_mandate(
    mandate_id: String,
    scheme: Option<String>,
    metadata: &HashMap<String, String>,
    payment_request_payment: Option<String>,
) -> PaymentMethodSnapshot {
    let payment_method_type = match scheme.as_deref() {
        Some("sepa_core") => PaymentMethodTypeEnum::DirectDebitSepa,
        Some("bacs") => PaymentMethodTypeEnum::DirectDebitBacs,
        Some("ach") => PaymentMethodTypeEnum::DirectDebitAch,
        // becs / becs_nz / pad / autogiro / betalingsservice / pay_to…: still a
        // direct-debit mandate (the rail gate keys on the provider), but with no
        // first-class enum variant yet — surfaces as `Other` in the UI.
        other => {
            log::warn!(
                "GoCardless mandate {mandate_id} has unmapped scheme {other:?}; storing as Other"
            );
            PaymentMethodTypeEnum::Other
        }
    };
    PaymentMethodSnapshot {
        external_payment_method_id: mandate_id,
        payment_method_type,
        // Mandates don't expose bank-account last4; would need a separate
        // customer_bank_account fetch.
        account_number_hint: None,
        card_brand: None,
        card_last4: None,
        card_exp_month: None,
        card_exp_year: None,
        meteroid_connection_id: metadata.get("meteroid.connection_id").cloned(),
        meteroid_customer_id: metadata.get("meteroid.customer_id").cloned(),
        meteroid_invoice_id: metadata.get("meteroid.invoice_id").cloned(),
        meteroid_checkout_session_id: metadata.get("meteroid.checkout_session_id").cloned(),
        payment_request_payment,
    }
}

/// Map the *initial* response of `POST /payments`. Settlement is asynchronous
/// so a fresh payment is almost always `Pending`; the final state arrives via
/// webhook.
fn payment_to_outcome(id: String, status: PaymentStatus, amount_minor: i64) -> ChargeOutcome {
    match status {
        PaymentStatus::Confirmed | PaymentStatus::PaidOut => {
            ChargeOutcome::Succeeded(ChargeReceipt {
                external_id: id,
                // The initial POST carries no settled amount; the requested
                // amount is the best figure until `paid_out` reconciles fees.
                amount_received_minor: amount_minor,
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
        PaymentStatus::Cancelled => ChargeOutcome::Cancelled(ChargeCancelled {
            external_id: Some(id),
            message: "Payment cancelled".to_string(),
            provider_request_id: None,
        }),
        PaymentStatus::CustomerApprovalDenied => ChargeOutcome::Failed(ChargeFailure {
            external_id: Some(id),
            code: Some(format!("{:?}", status).to_lowercase()),
            message: "Customer approval denied".to_string(),
            retryable: false,
            decline_kind: DeclineKind::Other,
            provider_request_id: None,
        }),
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
        PaymentStatus::Confirmed | PaymentStatus::PaidOut => RemoteTransactionStatus::Succeeded {
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

/// Which GoCardless operation produced an error, so a 4xx maps to the right
/// `ConnectorError` variant. Previously every failure was reported as
/// `Charge failed`, which was actively misleading — a `create_customer`
/// validation error looked like a failed payment in the logs.
#[derive(Clone, Copy, Debug)]
enum GcOp {
    Customer,
    Mandate,
    PaymentMethod,
    Charge,
    Refund,
}

impl GcOp {
    /// Classify a logical (4xx) failure into the matching semantic variant.
    fn logical_error(self, msg: String) -> ConnectorError {
        match self {
            GcOp::Customer => ConnectorError::CustomerOp(msg),
            GcOp::Mandate => ConnectorError::MandateSetup(msg),
            // Fetching a mandate/payment method is a read on the customer's setup.
            GcOp::PaymentMethod => ConnectorError::CustomerOp(msg),
            GcOp::Charge => ConnectorError::Charge(msg),
            GcOp::Refund => ConnectorError::Refund(msg),
        }
    }
}

fn map_gc_error(op: GcOp, e: GoCardlessError) -> Report<ConnectorError> {
    match e {
        GoCardlessError::Timeout => {
            Report::new(ConnectorError::Transport("gocardless timeout".into()))
        }
        GoCardlessError::ClientError(msg) => Report::new(ConnectorError::Transport(msg)),
        GoCardlessError::Api(req_err) if req_err.http_status >= 500 => {
            Report::new(ConnectorError::Transport(format!(
                "gocardless 5xx: {}",
                format_gc_request_error(&req_err)
            )))
        }
        GoCardlessError::Api(req_err) => {
            // 4xx validation/state error, not retryable. The top-level `message`
            // is generic ("Validation failed"); the actionable per-field reasons
            // live in `errors[]`, so surface them (plus request_id for support).
            Report::new(op.logical_error(format!(
                "gocardless rejected: {}",
                format_gc_request_error(&req_err)
            )))
        }
        GoCardlessError::Encode(e) => {
            Report::new(ConnectorError::Configuration(format!("gocardless: {e}")))
        }
    }
}

/// Render a GoCardless API error into a single actionable line. The top-level
/// `message` alone is uninformative (e.g. "Validation failed"); the per-field
/// entries in `errors[]` carry the real cause, so append them along with the
/// `request_id` (needed when escalating to GoCardless support).
fn format_gc_request_error(err: &gocardless_client::error::RequestError) -> String {
    let mut out = err
        .message
        .clone()
        .unwrap_or_else(|| "unknown error".into());

    if !err.errors.is_empty() {
        let details = err
            .errors
            .iter()
            .map(|d| {
                let field = d.field.as_deref();
                let msg = d.message.as_deref().or(d.reason.as_deref());
                match (field, msg) {
                    (Some(f), Some(m)) => format!("{f}: {m}"),
                    (Some(f), None) => f.to_string(),
                    (None, Some(m)) => m.to_string(),
                    (None, None) => "unspecified".to_string(),
                }
            })
            .collect::<Vec<_>>()
            .join("; ");
        out.push_str(&format!(" [{details}]"));
    }

    if let Some(request_id) = &err.request_id {
        out.push_str(&format!(" (request_id={request_id})"));
    }

    out
}

fn normalize_event(event: gocardless_client::webhook::Event) -> Option<NormalizedWebhookEvent> {
    let kind = match event.resource_type.as_str() {
        ev_resource::PAYMENTS => normalize_payment_event(&event),
        ev_resource::MANDATES => normalize_mandate_event(&event),
        ev_resource::BILLING_REQUESTS => normalize_billing_request_event(&event),
        ev_resource::REFUNDS => normalize_refund_event(&event),
        _ => Some(NormalizedEventKind::Acknowledged {
            reason: "unhandled gocardless resource_type",
        }),
    }?;

    let occurred_at = chrono::DateTime::parse_from_rfc3339(&event.created_at)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    // Only payment events echo `meteroid.tenant_id`; mandate/BR events don't
    // (the 3-property metadata cap has no room), so they get None and fall
    // through to normal handling.
    let owner_tenant_id = event
        .resource_metadata
        .get("meteroid.tenant_id")
        .or_else(|| event.metadata.get("meteroid.tenant_id"))
        .cloned();

    Some(NormalizedWebhookEvent {
        provider_event_id: event.id.clone(),
        provider_event_type: format!("{}.{}", event.resource_type, event.action),
        occurred_at,
        kind,
        owner_tenant_id,
    })
}

fn normalize_payment_event(
    event: &gocardless_client::webhook::Event,
) -> Option<NormalizedEventKind> {
    let payment_id = event.links.payment.clone()?;
    // Bank-triggered payment events carry our id in `resource_metadata` (the
    // payment's own metadata), not `metadata` (the triggering request's). Prefer
    // the former; the provider-id correlation in the handler still backstops.
    let meteroid_tx = event
        .resource_metadata
        .get("meteroid.transaction_id")
        .or_else(|| event.metadata.get("meteroid.transaction_id"))
        .cloned();
    Some(match event.action.as_str() {
        ev_action::CONFIRMED | ev_action::PAID_OUT => {
            NormalizedEventKind::PaymentSucceeded(PaymentSucceededEvent {
                external_transaction_id: payment_id,
                // Webhook carries no amount; the local transaction holds the
                // requested amount (fees arrive via a separate payout event).
                amount_received_minor: 0,
                currency: String::new(),
                meteroid_transaction_id: meteroid_tx,
            })
        }
        // Funds reclaimed by the bank after the payment already settled. On a
        // still-settled transaction this is a reversal (invoice reopens); if the
        // charge was never confirmed the reversal handler falls back to the
        // failed path. Same for `charged_back` below.
        ev_action::LATE_FAILURE_SETTLED => {
            NormalizedEventKind::PaymentReversed(PaymentReversedEvent {
                external_transaction_id: payment_id,
                meteroid_transaction_id: meteroid_tx,
                reason: "late_failure_settled".into(),
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
        ev_action::CHARGED_BACK => NormalizedEventKind::PaymentReversed(PaymentReversedEvent {
            external_transaction_id: payment_id,
            meteroid_transaction_id: meteroid_tx,
            reason: "charged_back".into(),
        }),
        // The bank cancelled the chargeback and returned the funds. Inverse of
        // `charged_back`: the handler restores the transaction and re-closes
        // the invoice.
        ev_action::CHARGEBACK_CANCELLED => {
            NormalizedEventKind::PaymentReinstated(PaymentReinstatedEvent {
                external_transaction_id: payment_id,
                meteroid_transaction_id: meteroid_tx,
                reason: "chargeback_cancelled".into(),
            })
        }
        _ => NormalizedEventKind::Acknowledged {
            reason: "unhandled gocardless payment action",
        },
    })
}

/// Outbound `refund()` is Unsupported, so any real GoCardless refund is
/// dashboard/API-initiated. Surface it so the invoice reflects the clawed-back
/// money instead of staying fully paid; the event carries no amounts, so the
/// handler fetches the refund + parent payment (`fetch_refund`).
fn normalize_refund_event(
    event: &gocardless_client::webhook::Event,
) -> Option<NormalizedEventKind> {
    let refund_id = event.links.refund.clone()?;
    Some(match event.action.as_str() {
        ev_action::CREATED | ev_action::PAID => NormalizedEventKind::RefundObserved {
            external_refund_id: refund_id,
        },
        // funds_returned/failed etc. — the cumulative fetch on created/paid
        // already reconciled (or will reconcile) the payment's refunded total.
        _ => NormalizedEventKind::Acknowledged {
            reason: "unhandled gocardless refund action",
        },
    })
}

fn normalize_mandate_event(
    event: &gocardless_client::webhook::Event,
) -> Option<NormalizedEventKind> {
    let mandate_id = event.links.mandate.clone()?;
    Some(match event.action.as_str() {
        // NOTE: attach is NOT handled here. It's driven by
        // `billing_requests.fulfilled` (see `normalize_billing_request_event`),
        // which lets us read our ids from the Billing Request metadata directly
        // rather than relying on GoCardless propagating them onto the mandate.
        // Mandate lifecycle events here only remove a method that dies.
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

/// `billing_requests.fulfilled` is the reliable, immediate signal that the
/// hosted flow completed and the mandate exists. We finalize by fetching the BR
/// (see `complete_mandate_setup`) — this is the canonical mandate-setup-done
/// event for both invoice payment and add-a-payment-method.
fn normalize_billing_request_event(
    event: &gocardless_client::webhook::Event,
) -> Option<NormalizedEventKind> {
    Some(match event.action.as_str() {
        ev_action::FULFILLED => {
            let billing_request_id = event.links.billing_request.clone()?;
            NormalizedEventKind::MandateSetupCompleted {
                provider_intent_id: billing_request_id,
            }
        }
        _ => NormalizedEventKind::Acknowledged {
            reason: "unhandled gocardless billing_request action",
        },
    })
}

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

    /// `payments.confirmed` must surface as `PaymentSucceeded` with the
    /// meteroid_transaction_id preserved from metadata.
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

    /// Real bank-triggered `payments.confirmed`: our id is in `resource_metadata`
    /// (the payment's own metadata), and `metadata` (the triggering request's) is
    /// empty. We must read it from `resource_metadata`.
    #[test]
    fn parse_event_payment_reads_resource_metadata() {
        let payload = br#"{
            "events":[{
                "id":"EV_RM_1",
                "created_at":"2026-05-19T12:00:00Z",
                "resource_type":"payments",
                "action":"confirmed",
                "links":{"payment":"PM_RM_1"},
                "metadata":{},
                "resource_metadata":{"meteroid.transaction_id":"tx_rm"}
            }]
        }"#;
        let parsed = GoCardlessConnector::new()
            .parse_event(&test_connector(), payload, &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::PaymentSucceeded(e) => {
                assert_eq!(e.meteroid_transaction_id.as_deref(), Some("tx_rm"));
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

    /// `billing_requests.fulfilled` → `MandateSetupCompleted` carrying the
    /// Billing Request id. The handler fetches that BR to recover our ids + the
    /// mandate (attach path is driven by fulfilled, NOT `mandates.active`, so we
    /// read metadata from the BR we own rather than relying on propagation).
    #[test]
    fn parse_event_billing_request_fulfilled() {
        let payload = br#"{
            "events":[{
                "id":"EV_BR_1",
                "created_at":"2026-05-19T12:00:00Z",
                "resource_type":"billing_requests",
                "action":"fulfilled",
                "links":{"billing_request":"BRQ_1","mandate_request_mandate":"MD_1"}
            }]
        }"#;
        let parsed = GoCardlessConnector::new()
            .parse_event(&test_connector(), payload, &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::MandateSetupCompleted { provider_intent_id } => {
                assert_eq!(provider_intent_id, "BRQ_1");
            }
            other => panic!("expected MandateSetupCompleted, got {other:?}"),
        }
    }

    /// `mandates.active` is no longer an attach trigger (fulfilled handles it);
    /// it's acknowledged as a no-op.
    #[test]
    fn parse_event_mandates_active_is_acknowledged() {
        let payload = br#"{
            "events":[{
                "id":"EV_MAND_1",
                "created_at":"2026-05-19T12:00:00Z",
                "resource_type":"mandates",
                "action":"active",
                "links":{"mandate":"MD_1","customer":"CU_1"},
                "metadata":{}
            }]
        }"#;
        let parsed = GoCardlessConnector::new()
            .parse_event(&test_connector(), payload, &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        assert!(
            matches!(parsed.kind, NormalizedEventKind::Acknowledged { .. }),
            "mandates.active should be acknowledged, got {:?}",
            parsed.kind
        );
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
        // `refund()` is Unsupported, so the capability must not advertise it.
        assert!(!caps.supports_refunds);
        assert!(!caps.supports_partial_refunds);
        assert_eq!(caps.mandate_setup_mode, MandateSetupMode::HostedRedirect);
        assert!(
            caps.supported_payment_methods
                .contains(&PaymentMethodTypeEnum::DirectDebitSepa)
        );
    }

    /// The realistic GoCardless shape: empty metadata everywhere, only
    /// `links.payment`. We must still surface the event (no meteroid id) so the
    /// handler can fall back to resolving by provider transaction id.
    #[test]
    fn parse_event_payments_confirmed_empty_metadata() {
        let payload = br#"{
            "events":[{
                "id":"EV_EMPTY_1",
                "created_at":"2026-05-19T12:00:00Z",
                "resource_type":"payments",
                "action":"confirmed",
                "links":{"payment":"PM_EMPTY_1"}
            }]
        }"#;
        let parsed = GoCardlessConnector::new()
            .parse_event(&test_connector(), payload, &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::PaymentSucceeded(e) => {
                assert_eq!(e.external_transaction_id, "PM_EMPTY_1");
                assert!(e.meteroid_transaction_id.is_none());
            }
            other => panic!("expected PaymentSucceeded, got {other:?}"),
        }
    }

    /// `customer_approval_granted` fires before the mandate is active; it must
    /// NOT attach/promote the mandate (attach is driven by
    /// `billing_requests.fulfilled`). Ack it instead.
    #[test]
    fn parse_event_mandates_customer_approval_granted_is_acked() {
        let payload = br#"{
            "events":[{
                "id":"EV_CAG_1",
                "created_at":"2026-05-19T12:00:00Z",
                "resource_type":"mandates",
                "action":"customer_approval_granted",
                "links":{"mandate":"MD_CAG","customer":"CU_CAG"},
                "metadata":{"meteroid.connection_id":"conn_x","meteroid.customer_id":"cust_x"}
            }]
        }"#;
        let parsed = GoCardlessConnector::new()
            .parse_event(&test_connector(), payload, &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::Acknowledged { .. } => {}
            other => panic!("expected Acknowledged, got {other:?}"),
        }
    }

    /// `payments.charged_back` on the real provider fixture must normalize to a
    /// `PaymentReversed` (not a plain failure) so the settlement pipeline reopens
    /// the invoice. The event carries empty metadata; the reversal handler falls
    /// back to the provider payment id.
    #[test]
    fn parse_event_payments_charged_back_is_reversal_real_fixture() {
        let payload = include_str!(
            "../../../../../tests/integration/fixtures/webhooks/gocardless/payments_charged_back.json"
        );
        let parsed = GoCardlessConnector::new()
            .parse_event(&test_connector(), payload.as_bytes(), &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::PaymentReversed(e) => {
                assert_eq!(e.external_transaction_id, "PM000TEST00003");
                assert_eq!(e.reason, "charged_back");
            }
            other => panic!("expected PaymentReversed, got {other:?}"),
        }
    }

    /// `payments.late_failure_settled` (funds reclaimed after settlement) must
    /// also normalize to `PaymentReversed`.
    #[test]
    fn parse_event_payments_late_failure_is_reversal_real_fixture() {
        let payload = include_str!(
            "../../../../../tests/integration/fixtures/webhooks/gocardless/payments_late_failure.json"
        );
        let parsed = GoCardlessConnector::new()
            .parse_event(&test_connector(), payload.as_bytes(), &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::PaymentReversed(e) => {
                assert_eq!(e.external_transaction_id, "PM000TEST00004");
                assert_eq!(e.reason, "late_failure_settled");
            }
            other => panic!("expected PaymentReversed, got {other:?}"),
        }
    }

    /// The initial `POST /payments` response for a rare synchronous confirm must
    /// carry the requested amount, not a literal 0.
    #[test]
    fn payment_to_outcome_carries_requested_amount() {
        match payment_to_outcome("PM_A".into(), PaymentStatus::Confirmed, 4_200) {
            ChargeOutcome::Succeeded(r) => assert_eq!(r.amount_received_minor, 4_200),
            other => panic!("expected Succeeded, got {other:?}"),
        }
    }

    /// `payments.chargeback_cancelled` (bank returned the clawed-back funds)
    /// must normalize to `PaymentReinstated` — the inverse of `charged_back` —
    /// so the handler restores the settlement and re-closes the invoice.
    #[test]
    fn parse_event_payments_chargeback_cancelled_is_reinstatement() {
        let payload = br#"{
            "events":[{
                "id":"EV_CBC_1",
                "created_at":"2026-05-19T12:00:00Z",
                "resource_type":"payments",
                "action":"chargeback_cancelled",
                "links":{"payment":"PM_CBC_1"},
                "details":{"origin":"bank","cause":"chargeback_cancelled"}
            }]
        }"#;
        let parsed = GoCardlessConnector::new()
            .parse_event(&test_connector(), payload, &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::PaymentReinstated(e) => {
                assert_eq!(e.external_transaction_id, "PM_CBC_1");
                assert_eq!(e.reason, "chargeback_cancelled");
            }
            other => panic!("expected PaymentReinstated, got {other:?}"),
        }
    }

    /// Dashboard-initiated refunds arrive as `refunds.*` events with no amounts;
    /// `created`/`paid` must surface as `RefundObserved` (the handler fetches the
    /// refund + parent payment), other refund actions are acknowledged.
    #[test]
    fn parse_event_refunds_created_is_refund_observed() {
        let payload = br#"{
            "events":[{
                "id":"EV_RF_1",
                "created_at":"2026-05-19T12:00:00Z",
                "resource_type":"refunds",
                "action":"created",
                "links":{"refund":"RF_1"}
            }]
        }"#;
        let parsed = GoCardlessConnector::new()
            .parse_event(&test_connector(), payload, &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::RefundObserved { external_refund_id } => {
                assert_eq!(external_refund_id, "RF_1");
            }
            other => panic!("expected RefundObserved, got {other:?}"),
        }

        let funds_returned = br#"{
            "events":[{
                "id":"EV_RF_2",
                "created_at":"2026-05-19T12:00:00Z",
                "resource_type":"refunds",
                "action":"funds_returned",
                "links":{"refund":"RF_2"}
            }]
        }"#;
        let parsed = GoCardlessConnector::new()
            .parse_event(&test_connector(), funds_returned, &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        assert!(
            matches!(parsed.kind, NormalizedEventKind::Acknowledged { .. }),
            "refunds.funds_returned should be acknowledged, got {:?}",
            parsed.kind
        );
    }
}
