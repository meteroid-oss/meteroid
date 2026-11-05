//! GoCardless connector. Mandate setup is hosted-redirect (no embedded SDK):
//! create a Billing Request, mint a Flow, redirect to `authorisation_url`, then
//! complete on return. Settlement is asynchronous — `POST /payments` returns
//! `pending_submission` and final state arrives 3–5 business days later via
//! webhook. Webhook endpoints are dashboard-managed; self-registration is
//! unsupported.

use super::account::{ValidatedCredentials, mask_secret, static_methods};
use super::connector::{
    ConnectorIdentity, ConnectorRef, CredentialOps, CustomerOps, HostedSetupCompletion,
    InboundWebhooks, MandateOps, MandateSetup, PaymentCapabilities, PaymentOps, ReconcileOps,
    RefundOps, RefundSupport, WebhookOps,
};
use super::error::{ConnectorError, CredentialsRejected, adoption_mismatch};
use super::events::{
    NormalizedEventKind, NormalizedEventSubscription, NormalizedWebhookEvent, PaymentFailedEvent,
    PaymentMethodDetachedEvent, PaymentReinstatedEvent, PaymentReversedEvent,
    PaymentSucceededEvent,
};
use super::model::{
    ChargeAcknowledged, ChargeCancelled, ChargeFailure, ChargeOutcome, ChargeReceipt,
    ChargeRequest, CreateCustomerRequest, DeclineKind, ExternalCustomerRef,
    MandateSetupInstruction, MandateSetupRequest, PaymentMethodSnapshot, RefundAcknowledged,
    RefundFailure, RefundOutcome, RefundReceipt, RefundRequest, RefundSnapshot, RegisteredWebhook,
    RemoteRefundStatus, RemoteTransactionStatus, WebhookDeliveryUnit,
};
use crate::domain::connectors::{
    GocardlessPublicData, GocardlessSensitiveData, MethodState, PaymentConnectorPublicData,
    PaymentMethodAvailability, ProviderAccount,
};
use crate::domain::enums::{PaymentProvider, ReversalKind};
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
use gocardless_client::refunds::{
    CreateRefund, CreateRefundLinks, Refund as GcRefund, RefundApi, RefundStatus as GcRefundStatus,
};
use gocardless_client::webhook::{
    EventEnvelope, GoCardlessWebhook, action as ev_action, resource_type as ev_resource,
};
use http::HeaderMap;
use secrecy::SecretString;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::OnceLock;

pub(super) const GOCARDLESS_CAPABILITIES: PaymentCapabilities = PaymentCapabilities {
    supported_payment_methods: &[
        PaymentMethodTypeEnum::DirectDebitSepa,
        PaymentMethodTypeEnum::DirectDebitAch,
        PaymentMethodTypeEnum::DirectDebitBacs,
    ],
    supports_mandates: true,
    refunds: RefundSupport::Partial,
    // Idempotency keys are honoured for at least 30 days.
    refund_replay_window: Some(std::time::Duration::from_secs(12 * 60 * 60)),
    charge_replay_window: std::time::Duration::from_secs(7 * 24 * 60 * 60),
    supports_3ds: false,
    supports_disputes: true,
    webhooks: Some(InboundWebhooks {
        requires_secret: true,
        // No public API to manage webhook endpoints; merchants configure them in
        // the dashboard and paste the signing secret into our connect form.
        self_registration: false,
        // No timestamp in the signature header, so this tolerance is not enforced
        // at the header level; replay protection is per-event (provider_config_id,
        // provider_event_id) DB dedup — the router splits each batched GoCardless
        // event into its own audit row keyed by the event's `EV...` id. Value
        // surfaced for capability honesty.
        replay_tolerance_secs: NonZeroU32::new(3600).unwrap(),
    }),
    asynchronous_settlement: true,
    mandate_setup: MandateSetup::HostedRedirect {
        // `billing_requests.fulfilled` completes a hosted setup even when the
        // return redirect is lost — no pending-intent persistence or sweep.
        completion: HostedSetupCompletion::WebhookBacked,
        invoice_payment: false,
    },
    supports_hosted_checkout: true,
    pending_charge_accepted: true,
    is_test_gateway: false,
};

/// GoCardless connector, bound to one connector's identity and credentials for
/// its whole life. Live + sandbox clients are static singletons so all tenants
/// share a connection pool to each environment.
#[derive(Debug, Clone)]
pub struct GoCardlessConnector {
    connector: ConnectorRef,
    public: GocardlessPublicData,
    sensitive: GocardlessSensitiveData,
}

impl GoCardlessConnector {
    pub fn new(
        connector: ConnectorRef,
        public: GocardlessPublicData,
        sensitive: GocardlessSensitiveData,
    ) -> Self {
        Self {
            connector,
            public,
            sensitive,
        }
    }

    fn client(&self) -> &'static GoCardlessClient {
        // Sandbox unless the environment is exactly `Live`: a misconfigured or
        // missing environment must never silently route to live and charge real
        // money (`GocardlessEnvironment` defaults to `Sandbox`, so an absent or
        // unreadable field in the stored blob lands on sandbox).
        if self.public.is_sandbox() {
            static SANDBOX: OnceLock<GoCardlessClient> = OnceLock::new();
            SANDBOX.get_or_init(GoCardlessClient::from_sandbox)
        } else {
            static LIVE: OnceLock<GoCardlessClient> = OnceLock::new();
            LIVE.get_or_init(GoCardlessClient::new)
        }
    }

    /// The bearer token this adapter was built with. The store refuses to store an empty one,
    /// and GoCardless answers 401 for a bad one, which surfaces as rejected credentials.
    fn access_token(&self) -> SecretString {
        self.sensitive.access_token.clone()
    }
}

impl ConnectorIdentity for GoCardlessConnector {
    fn provider(&self) -> PaymentProvider {
        PaymentProvider::Gocardless
    }
    fn capabilities(&self) -> &PaymentCapabilities {
        &GOCARDLESS_CAPABILITIES
    }
}

#[async_trait]
impl CredentialOps for GoCardlessConnector {
    /// Cheap authenticated GET: catches a wrong token or environment before it is persisted.
    /// The creditor doubles as the account: its schemes say which debits are enabled.
    async fn validate_credentials(&self) -> Result<ValidatedCredentials, Report<ConnectorError>> {
        use gocardless_client::creditors::CreditorApi;
        let token = self.access_token();
        let public = PaymentConnectorPublicData::Gocardless(self.public.clone());
        let creditor_id = self
            .public
            .creditor_id
            .clone()
            .filter(|c| !c.trim().is_empty());
        let livemode = !self.public.is_sandbox();
        let client = self.client();
        // The same probe as before account detection: it alone decides whether the token works.
        client
            .validate_credentials(&token)
            .await
            .map_err(|e| match &e {
                GoCardlessError::Api(req) if matches!(req.http_status, 400 | 401 | 403) => {
                    let report = Report::new(ConnectorError::Configuration(
                        "Invalid access token".to_string(),
                    ))
                    .attach(e.to_string());
                    if req.http_status == 400 {
                        report
                    } else {
                        report.attach_opaque(CredentialsRejected)
                    }
                }
                _ => Report::new(ConnectorError::Transport(format!(
                    "Couldn't reach GoCardless to verify the token: {e}"
                ))),
            })?;
        let creditors = match &creditor_id {
            Some(id) => client.get_creditor(id, &token).await.map(|c| vec![c]),
            None => client.list_creditors(&token).await,
        };
        let creditors = match creditors {
            Ok(creditors) => creditors,
            Err(GoCardlessError::Api(req)) if req.http_status == 404 && creditor_id.is_some() => {
                return Err(Report::new(ConnectorError::Configuration(
                    "GoCardless has no creditor with this ID on this account.".into(),
                )));
            }
            Err(GoCardlessError::Api(req)) if req.http_status == 403 => {
                // The token works but can't read creditors: keep it usable, with what we know.
                log::warn!("GoCardless token can't read creditors; account details unknown");
                return Ok(ValidatedCredentials {
                    account: ProviderAccount {
                        // The token can't read creditors, so there is no
                        // creditor id to record yet.
                        account_id: None,
                        display_name: None,
                        livemode,
                        key_hint: Some(mask_secret(secrecy::ExposeSecret::expose_secret(&token))),
                        methods: static_methods(
                            GOCARDLESS_CAPABILITIES.supported_payment_methods,
                            MethodState::Enabled,
                        ),
                        checked_at: chrono::Utc::now().naive_utc(),
                    },
                    public,
                });
            }
            Err(e) => {
                return Err(Report::new(ConnectorError::Transport(format!(
                    "Couldn't read the GoCardless creditor: {e}"
                ))));
            }
        };
        // Without a creditor id, the adapter can only use a token with a single creditor.
        let creditor = match creditors.as_slice() {
            [single] => single,
            [] => {
                return Err(Report::new(ConnectorError::Configuration(
                    "This GoCardless account has no creditor yet. Finish onboarding in GoCardless first."
                        .into(),
                )));
            }
            _ => {
                return Err(Report::new(ConnectorError::Configuration(
                    "This GoCardless account has several creditors. Enter the creditor ID to use."
                        .into(),
                )));
            }
        };
        let schemes: Vec<&str> = creditor
            .scheme_identifiers
            .iter()
            .map(|s| s.scheme.as_str())
            .collect();
        Ok(ValidatedCredentials {
            account: ProviderAccount {
                account_id: Some(creditor.id.clone()),
                display_name: creditor.name.clone(),
                livemode,
                key_hint: Some(mask_secret(secrecy::ExposeSecret::expose_secret(&token))),
                methods: gocardless_methods(&schemes),
                checked_at: chrono::Utc::now().naive_utc(),
            },
            public,
        })
    }
}

/// A debit rail is enabled when the creditor has an identifier for its scheme.
fn gocardless_methods(schemes: &[&str]) -> Vec<PaymentMethodAvailability> {
    GOCARDLESS_CAPABILITIES
        .supported_payment_methods
        .iter()
        .map(|method| {
            let (scheme_names, label): (&[&str], &str) = match method {
                PaymentMethodTypeEnum::DirectDebitSepa => (&["sepa_core", "sepa_cor1"], "SEPA"),
                PaymentMethodTypeEnum::DirectDebitAch => (&["ach"], "ACH"),
                PaymentMethodTypeEnum::DirectDebitBacs => (&["bacs"], "Bacs"),
                _ => (&[], ""),
            };
            let enabled = schemes.iter().any(|s| scheme_names.contains(s));
            PaymentMethodAvailability {
                method: method.clone(),
                state: if enabled {
                    MethodState::Enabled
                } else {
                    MethodState::Unavailable
                },
                reason: (!enabled)
                    .then(|| format!("{label} isn't enabled on this GoCardless creditor")),
            }
        })
        .collect()
}

#[async_trait]
impl CustomerOps for GoCardlessConnector {
    async fn create_customer(
        &self,
        customer: &Customer,
        request: CreateCustomerRequest,
    ) -> Result<ExternalCustomerRef, Report<ConnectorError>> {
        let token = self.access_token();
        let client = self.client();

        // GoCardless requires given+family name unless a company_name is set.
        let (given_name, family_name, company_name) = match customer.person_name() {
            Some((first, last)) => (Some(first.to_string()), Some(last.to_string()), None),
            None => (None, None, Some(customer.name.clone())),
        };
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
                    given_name,
                    family_name,
                    company_name,
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
        connection: &CustomerConnection,
        request: MandateSetupRequest<'_>,
    ) -> Result<MandateSetupInstruction, Report<ConnectorError>> {
        let token = self.access_token();
        let client = self.client();

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

        let creditor_id = self.public.creditor_id.clone();

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
        external_payment_method_id: &str,
        _external_customer_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        let token = self.access_token();
        let client = self.client();
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
        intent_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        let token = self.access_token();
        let client = self.client();

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
        request: ChargeRequest<'_>,
    ) -> Result<ChargeOutcome, Report<ConnectorError>> {
        let token = self.access_token();
        let client = self.client();

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
                self.connector.tenant_id.as_base62(),
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
            // A 409 replay returns the payment the key first created, which may differ.
            Ok(payment)
                if payment.amount != request.amount_minor
                    || !payment.currency.eq_ignore_ascii_case(request.currency) =>
            {
                Err(adoption_mismatch(
                    &payment.id,
                    payment.amount,
                    &payment.currency,
                    payment_to_outcome(payment.id.clone(), payment.status.clone(), payment.amount),
                    &request,
                ))
            }
            Ok(payment) => Ok(payment_to_outcome(
                payment.id,
                payment.status,
                request.amount_minor,
            )),
            Err(e) => Err(map_gc_error(GcOp::Charge, e)),
        }
    }
}

/// No unconfirmed intents: wallet top-ups are offered on providers that can pin one.
impl super::connector::WalletOps for GoCardlessConnector {}

#[async_trait]
impl RefundOps for GoCardlessConnector {
    async fn refund(
        &self,
        request: RefundRequest<'_>,
    ) -> Result<RefundOutcome, Report<ConnectorError>> {
        let token = self.access_token();
        let client = self.client();

        // A replay after a lost response would recompute a different `total_amount_confirmation`,
        // which GoCardless may reject before it looks at the idempotency key: find the refund
        // that already went through by our own id instead.
        let ours = request
            .metadata
            .iter()
            .find(|(k, _)| k == "meteroid.refund_id")
            .map(|(_, v)| v.clone());
        if let Some(ours) = ours {
            let existing = client
                .list_payment_refunds(request.external_transaction_id, &token)
                .await
                .map_err(|e| map_gc_error(GcOp::Refund, e))?
                .into_iter()
                .find(|r| r.metadata.get("meteroid.refund_id") == Some(&ours));
            if let Some(existing) = existing {
                return Ok(refund_outcome(existing));
            }
        }

        // `total_amount_confirmation` must match GoCardless' own running total, which also counts
        // dashboard refunds we may not have recorded yet.
        let payment = client
            .get_payment(request.external_transaction_id, &token)
            .await
            .map_err(|e| map_gc_error(GcOp::Refund, e))?;

        let refund = client
            .create_refund(
                CreateRefund {
                    amount: request.amount_minor,
                    total_amount_confirmation: payment.amount_refunded + request.amount_minor,
                    reference: None,
                    metadata: Some(request.metadata.into_iter().collect()),
                    links: CreateRefundLinks {
                        payment: request.external_transaction_id.to_string(),
                    },
                },
                &token,
                request.idempotency_key.as_str(),
            )
            .await
            .map_err(|e| map_gc_error(GcOp::Refund, e))?;

        Ok(refund_outcome(refund))
    }

    /// Read the refund for its status and parent payment, then the payment for its cumulative
    /// refunded total: `refunds.*` events carry no amounts.
    async fn fetch_refund(
        &self,
        external_refund_id: &str,
        _external_transaction_id: Option<&str>,
    ) -> Result<RefundSnapshot, Report<ConnectorError>> {
        let token = self.access_token();
        let client = self.client();

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
            status: remote_refund_status(&refund.status),
            external_refund_id: refund.id,
            amount_minor: refund.amount,
            cumulative_refunded_minor: Some(payment.amount_refunded),
            currency: payment.currency,
        })
    }
}

/// `bounced` and `funds_returned` both mean the customer never kept the money.
fn remote_refund_status(status: &GcRefundStatus) -> RemoteRefundStatus {
    let returned = |code: &str| RemoteRefundStatus::Failed {
        code: Some(code.to_string()),
        message: "The refund did not reach the customer; the funds were returned".to_string(),
    };
    match status {
        GcRefundStatus::Paid => RemoteRefundStatus::Succeeded,
        GcRefundStatus::Cancelled => RemoteRefundStatus::Canceled,
        GcRefundStatus::Bounced => returned("bounced"),
        GcRefundStatus::FundsReturned => returned("funds_returned"),
        GcRefundStatus::Created
        | GcRefundStatus::PendingSubmission
        | GcRefundStatus::Submitted
        | GcRefundStatus::Unknown => RemoteRefundStatus::Pending,
    }
}

fn refund_outcome(refund: GcRefund) -> RefundOutcome {
    match remote_refund_status(&refund.status) {
        RemoteRefundStatus::Succeeded => RefundOutcome::Succeeded(RefundReceipt {
            external_refund_id: refund.id,
            amount_refunded_minor: refund.amount,
            processed_at: chrono::Utc::now().naive_utc(),
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
            code: Some("cancelled".to_string()),
            message: "Cancelled by GoCardless".to_string(),
            provider_request_id: None,
        }),
    }
}

#[async_trait]
impl ReconcileOps for GoCardlessConnector {
    async fn fetch_transaction_status(
        &self,
        external_transaction_id: &str,
    ) -> Result<RemoteTransactionStatus, Report<ConnectorError>> {
        let token = self.access_token();
        let client = self.client();

        let result = client.get_payment(external_transaction_id, &token).await;
        match result {
            Ok(payment) => Ok(remote_status_from_payment(
                payment.status,
                payment.amount,
                payment.currency,
            )),
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
        _url: &str,
        _subscriptions: &[NormalizedEventSubscription],
    ) -> Result<RegisteredWebhook, Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: self.provider().into(),
            capability: "webhook.register (GoCardless endpoints are dashboard-managed)",
        }))
    }

    async fn unregister_webhook(&self, _endpoint_id: &str) -> Result<(), Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: self.provider().into(),
            capability: "webhook.unregister",
        }))
    }

    async fn sync_webhook_events(
        &self,
        _endpoint_id: &str,
        _subscriptions: &[NormalizedEventSubscription],
    ) -> Result<(), Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: self.provider().into(),
            capability: "webhook.sync",
        }))
    }

    fn verify_signature(
        &self,
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
        payload: &[u8],
        headers: &HeaderMap,
    ) -> Result<Option<NormalizedWebhookEvent>, Report<ConnectorError>> {
        Ok(self.parse_events(payload, headers)?.into_iter().next())
    }

    /// GoCardless batches several events per POST; return all (dropping any
    /// loses it permanently once we ACK 200).
    /// A `{"events":[...]}` batch becomes one unit per inner event, keyed by its `EV...` id, so
    /// dedup is per event and a bad event doesn't block the others.
    fn split_delivery(
        &self,
        payload: &[u8],
    ) -> Result<Vec<WebhookDeliveryUnit>, Report<ConnectorError>> {
        let json: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| Report::new(ConnectorError::PayloadDecode(e.to_string())))?;
        let Some(events) = json.get("events").and_then(|v| v.as_array()) else {
            return Ok(vec![WebhookDeliveryUnit {
                event_id: json.get("id").and_then(|v| v.as_str()).map(str::to_string),
                body: payload.to_vec(),
            }]);
        };
        events
            .iter()
            .map(|ev| {
                let body = serde_json::to_vec(&serde_json::json!({ "events": [ev] }))
                    .map_err(|e| Report::new(ConnectorError::PayloadDecode(e.to_string())))?;
                Ok(WebhookDeliveryUnit {
                    event_id: ev.get("id").and_then(|v| v.as_str()).map(str::to_string),
                    body,
                })
            })
            .collect()
    }

    fn parse_events(
        &self,
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
        fingerprint: None,
        meteroid_connection_id: metadata.get("meteroid.connection_id").cloned(),
        meteroid_customer_id: metadata.get("meteroid.customer_id").cloned(),
        meteroid_invoice_id: metadata.get("meteroid.invoice_id").cloned(),
        meteroid_checkout_session_id: metadata.get("meteroid.checkout_session_id").cloned(),
        meteroid_transaction_id: metadata.get("meteroid.transaction_id").cloned(),
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

fn remote_status_from_payment(
    status: PaymentStatus,
    amount: i64,
    currency: String,
) -> RemoteTransactionStatus {
    match status {
        PaymentStatus::Confirmed | PaymentStatus::PaidOut => RemoteTransactionStatus::Succeeded {
            amount_received_minor: amount,
            currency,
            processed_at: chrono::Utc::now().naive_utc(),
        },
        PaymentStatus::PendingSubmission | PaymentStatus::Submitted => {
            RemoteTransactionStatus::Processing
        }
        PaymentStatus::PendingCustomerApproval | PaymentStatus::Unknown => {
            RemoteTransactionStatus::Pending
        }
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
            let rejected_key = matches!(req_err.http_status, 401 | 403);
            let report = Report::new(op.logical_error(format!(
                "gocardless rejected: {}",
                format_gc_request_error(&req_err)
            )));
            if rejected_key {
                report.attach_opaque(CredentialsRejected)
            } else {
                report
            }
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

/// Map the GoCardless `details.cause` code on a `charged_back` /
/// `late_failure_settled` event to a typed [`ReversalKind`]. Unrecognized
/// causes (including none at all) fall back to `Other`; the raw cause is kept
/// alongside for support.
fn decline_kind_from_cause(cause: Option<&str>) -> DeclineKind {
    match cause {
        Some("insufficient_funds") => DeclineKind::InsufficientFunds,
        Some(
            "mandate_cancelled"
            | "mandate_expired"
            | "direct_debit_not_enabled"
            | "invalid_bank_details"
            | "bank_account_closed"
            | "bank_account_transferred",
        ) => DeclineKind::MandateInactive,
        _ => DeclineKind::Other,
    }
}

fn reversal_kind_from_cause(cause: Option<&str>) -> ReversalKind {
    match cause {
        Some("refund_requested" | "authorisation_disputed" | "customer_disputed") => {
            ReversalKind::DebtorRecall
        }
        Some("chargeback") => ReversalKind::Chargeback,
        Some("insufficient_funds") => ReversalKind::InsufficientFunds,
        Some(
            "mandate_cancelled"
            | "direct_debit_not_enabled"
            | "invalid_bank_details"
            | "mandate_expired",
        ) => ReversalKind::MandateInvalid,
        Some(
            "bank_account_closed"
            | "bank_account_transferred"
            | "account_blocked_for_any_financial_transaction"
            | "payment_stopped",
        ) => ReversalKind::Returned,
        _ => ReversalKind::Other,
    }
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
        // A failed debit the bank reported after the payment settled: a reversal that reopens
        // the invoice; if the charge was never confirmed the handler falls back to the failed
        // path. `charged_back` below is the payer's claim instead: the invoice stays paid.
        ev_action::LATE_FAILURE_SETTLED => {
            let cause = event.details.as_ref().and_then(|d| d.cause.as_deref());
            NormalizedEventKind::PaymentReversed(PaymentReversedEvent {
                external_transaction_id: payment_id,
                meteroid_transaction_id: meteroid_tx,
                reason: "late_failure_settled".into(),
                kind: reversal_kind_from_cause(cause),
                // The bank reported the failed debit late: nothing was paid, the invoice reopens.
                disputed: false,
                cause: cause.map(str::to_string),
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
            decline_kind: decline_kind_from_cause(
                event.details.as_ref().and_then(|d| d.cause.as_deref()),
            ),
            meteroid_transaction_id: meteroid_tx,
        }),
        ev_action::CANCELLED => NormalizedEventKind::PaymentFailed(PaymentFailedEvent {
            external_transaction_id: payment_id,
            code: Some("cancelled".into()),
            message: "Payment cancelled".into(),
            retryable: false,
            decline_kind: DeclineKind::Other,
            meteroid_transaction_id: meteroid_tx,
        }),
        ev_action::CHARGED_BACK => {
            let cause = event.details.as_ref().and_then(|d| d.cause.as_deref());
            NormalizedEventKind::PaymentReversed(PaymentReversedEvent {
                external_transaction_id: payment_id,
                meteroid_transaction_id: meteroid_tx,
                reason: "charged_back".into(),
                // A Direct Debit Guarantee or indemnity claim: always the payer's.
                kind: match reversal_kind_from_cause(cause) {
                    ReversalKind::Other => ReversalKind::Chargeback,
                    kind => kind,
                },
                disputed: true,
                cause: cause.map(str::to_string),
            })
        }
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

/// The event carries no amounts: the handler resolves the refund and its payment
/// (`fetch_refund`), settling or failing a refund we issued and recording a dashboard one.
fn normalize_refund_event(
    event: &gocardless_client::webhook::Event,
) -> Option<NormalizedEventKind> {
    let refund_id = event.links.refund.clone()?;
    Some(match event.action.as_str() {
        ev_action::CREATED | ev_action::PAID | ev_action::FAILED | ev_action::FUNDS_RETURNED => {
            NormalizedEventKind::RefundObserved {
                external_refund_id: refund_id,
            }
        }
        // `refund_settled` is the payout deduction: no change for the customer.
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
    use super::super::connector::MandateSetupMode;
    use super::*;
    use crate::domain::connectors::{
        Connector, CredentialState, GocardlessEnvironment, GocardlessPublicData,
        GocardlessSensitiveData,
    };
    use crate::domain::connectors::{
        ConnectorConfig, PaymentConnectorConfig, PaymentProviderConfig,
    };

    use chrono::NaiveDateTime;
    use common_domain::ids::{ConnectorId, TenantId};
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    const TEST_SECRET: &str = "gc_whsec_test_unit";

    fn test_connector() -> Connector {
        Connector {
            status: crate::domain::enums::ConnectorStatusEnum::Active,
            credentials: CredentialState::Unverified,
            webhook_route: None,
            id: ConnectorId::new(),
            created_at: NaiveDateTime::default(),
            tenant_id: TenantId::new(),
            alias: "gc-test".into(),
            config: ConnectorConfig::Payment(PaymentConnectorConfig::configured(
                PaymentProviderConfig::Gocardless(test_public(), test_sensitive()),
            )),
        }
    }

    fn test_public() -> GocardlessPublicData {
        GocardlessPublicData {
            creditor_id: Some("CR000".into()),
            environment: GocardlessEnvironment::Sandbox,
        }
    }

    fn test_sensitive() -> GocardlessSensitiveData {
        GocardlessSensitiveData {
            access_token: "sandbox_token".into(),
            webhook_secret: TEST_SECRET.into(),
        }
    }

    /// The adapter under test, bound to the fixture connector's identity and
    /// credentials the way the factory builds it.
    fn test_adapter() -> GoCardlessConnector {
        let connector = test_connector();
        GoCardlessConnector::new(
            ConnectorRef::from(&connector),
            test_public(),
            test_sensitive(),
        )
    }

    /// A batch splits into one unit per inner event, keyed by that event's id.
    #[test]
    fn batch_splits_into_one_unit_per_event() {
        let body = br#"{"events":[{"id":"EV1","resource_type":"payments","action":"confirmed","links":{}},{"id":"EV2","resource_type":"payments","action":"failed","links":{}}]}"#;
        let units = test_adapter().split_delivery(body).unwrap();
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].event_id.as_deref(), Some("EV1"));
        assert_eq!(units[1].event_id.as_deref(), Some("EV2"));
        let first: serde_json::Value = serde_json::from_slice(&units[0].body).unwrap();
        assert_eq!(first["events"].as_array().unwrap().len(), 1);
        assert_eq!(first["events"][0]["id"], "EV1");
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
        let result = test_adapter().verify_signature(
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
        let result = test_adapter().verify_signature(
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
        let result = test_adapter().verify_signature(
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
        let parsed = test_adapter()
            .parse_event(payload, &HeaderMap::new())
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
        let parsed = test_adapter()
            .parse_event(payload, &HeaderMap::new())
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
        let parsed = test_adapter()
            .parse_event(payload, &HeaderMap::new())
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
        let parsed = test_adapter()
            .parse_event(payload, &HeaderMap::new())
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
        let parsed = test_adapter()
            .parse_event(payload, &HeaderMap::new())
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
        let parsed = test_adapter()
            .parse_event(payload, &HeaderMap::new())
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
        let adapter = test_adapter();
        let caps = adapter.capabilities();
        assert!(!caps.supports_cards());
        assert!(caps.supports_mandates);
        assert!(caps.asynchronous_settlement);
        assert!(!caps.supports_self_webhook_registration());
        assert!(caps.supports_partial_refunds());
        assert!(caps.refund_replay_window.is_some());
        assert_eq!(caps.mandate_setup_mode(), MandateSetupMode::HostedRedirect);
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
        let parsed = test_adapter()
            .parse_event(payload, &HeaderMap::new())
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
        let parsed = test_adapter()
            .parse_event(payload, &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::Acknowledged { .. } => {}
            other => panic!("expected Acknowledged, got {other:?}"),
        }
    }

    /// `payments.charged_back` on the real provider fixture is the payer's claim: a disputed
    /// `PaymentReversed` that keeps the invoice paid. The event carries empty metadata; the
    /// reversal handler falls back to the provider payment id.
    #[test]
    fn parse_event_payments_charged_back_is_reversal_real_fixture() {
        let payload = include_str!(
            "../../../../../tests/integration/fixtures/webhooks/gocardless/payments_charged_back.json"
        );
        let parsed = test_adapter()
            .parse_event(payload.as_bytes(), &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::PaymentReversed(e) => {
                assert_eq!(e.external_transaction_id, "PM000TEST00003");
                assert_eq!(e.reason, "charged_back");
                assert_eq!(e.kind, ReversalKind::Chargeback);
                assert!(e.disputed, "a chargeback keeps the invoice paid");
                assert_eq!(e.cause.as_deref(), Some("chargeback"));
            }
            other => panic!("expected PaymentReversed, got {other:?}"),
        }
    }

    /// `payments.late_failure_settled` (a failed debit reported after settlement) is a failed
    /// collection: a `PaymentReversed` that reopens the invoice.
    #[test]
    fn parse_event_payments_late_failure_is_reversal_real_fixture() {
        let payload = include_str!(
            "../../../../../tests/integration/fixtures/webhooks/gocardless/payments_late_failure.json"
        );
        let parsed = test_adapter()
            .parse_event(payload.as_bytes(), &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::PaymentReversed(e) => {
                assert_eq!(e.external_transaction_id, "PM000TEST00004");
                assert_eq!(e.reason, "late_failure_settled");
                assert_eq!(e.kind, ReversalKind::Other);
                assert!(!e.disputed, "a late failure reopens the invoice");
                assert_eq!(e.cause.as_deref(), Some("late_failure_settled"));
            }
            other => panic!("expected PaymentReversed, got {other:?}"),
        }
    }

    #[test]
    fn decline_kind_from_cause_flags_dead_mandates() {
        assert_eq!(
            decline_kind_from_cause(Some("insufficient_funds")),
            DeclineKind::InsufficientFunds
        );
        assert_eq!(
            decline_kind_from_cause(Some("mandate_cancelled")),
            DeclineKind::MandateInactive
        );
        assert_eq!(
            decline_kind_from_cause(Some("bank_account_closed")),
            DeclineKind::MandateInactive
        );
        assert_eq!(decline_kind_from_cause(Some("other")), DeclineKind::Other);
        assert_eq!(decline_kind_from_cause(None), DeclineKind::Other);
    }

    #[test]
    fn reversal_kind_from_cause_maps_known_causes() {
        assert_eq!(
            reversal_kind_from_cause(Some("refund_requested")),
            ReversalKind::DebtorRecall
        );
        assert_eq!(
            reversal_kind_from_cause(Some("authorisation_disputed")),
            ReversalKind::DebtorRecall
        );
        assert_eq!(
            reversal_kind_from_cause(Some("chargeback")),
            ReversalKind::Chargeback
        );
        assert_eq!(
            reversal_kind_from_cause(Some("insufficient_funds")),
            ReversalKind::InsufficientFunds
        );
        assert_eq!(
            reversal_kind_from_cause(Some("mandate_cancelled")),
            ReversalKind::MandateInvalid
        );
        assert_eq!(
            reversal_kind_from_cause(Some("invalid_bank_details")),
            ReversalKind::MandateInvalid
        );
        assert_eq!(
            reversal_kind_from_cause(Some("bank_account_closed")),
            ReversalKind::Returned
        );
        assert_eq!(
            reversal_kind_from_cause(Some("payment_stopped")),
            ReversalKind::Returned
        );
        assert_eq!(
            reversal_kind_from_cause(Some("something_new")),
            ReversalKind::Other
        );
        assert_eq!(reversal_kind_from_cause(None), ReversalKind::Other);
    }

    /// A `charged_back` with a cause we do not map is still the payer's claim.
    #[test]
    fn a_charge_back_with_an_unknown_cause_is_a_disputed_chargeback() {
        let payload = br#"{
            "events":[{
                "id":"EV_CB_X",
                "created_at":"2026-05-19T12:00:00Z",
                "resource_type":"payments",
                "action":"charged_back",
                "links":{"payment":"PM_CB_X"},
                "details":{"origin":"bank","cause":"return_on_odfi_request"}
            }]
        }"#;
        let parsed = test_adapter()
            .parse_event(payload, &HeaderMap::new())
            .expect("parse ok")
            .expect("event surfaced");
        match parsed.kind {
            NormalizedEventKind::PaymentReversed(e) => {
                assert_eq!(e.kind, ReversalKind::Chargeback);
                assert!(e.disputed);
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
    /// so the handler restores the settlement.
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
        let parsed = test_adapter()
            .parse_event(payload, &HeaderMap::new())
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

    /// `refunds.*` events carry no amounts: every action that moves the customer's money surfaces
    /// as `RefundObserved` for the handler to resolve; the payout deduction is acknowledged.
    #[test]
    fn parse_event_refund_actions() {
        let parse = |action: &str| {
            let payload = format!(
                r#"{{"events":[{{"id":"EV_RF","created_at":"2026-05-19T12:00:00Z",
                    "resource_type":"refunds","action":"{action}","links":{{"refund":"RF_1"}}}}]}}"#
            );
            test_adapter()
                .parse_event(payload.as_bytes(), &HeaderMap::new())
                .expect("parse ok")
                .expect("event surfaced")
                .kind
        };
        for action in ["created", "paid", "failed", "funds_returned"] {
            match parse(action) {
                NormalizedEventKind::RefundObserved { external_refund_id } => {
                    assert_eq!(external_refund_id, "RF_1");
                }
                other => panic!("refunds.{action}: expected RefundObserved, got {other:?}"),
            }
        }
        assert!(matches!(
            parse("refund_settled"),
            NormalizedEventKind::Acknowledged { .. }
        ));
    }

    #[test]
    fn refund_status_maps_to_the_remote_outcome() {
        use gocardless_client::refunds::RefundStatus as S;
        for pending in [S::Created, S::PendingSubmission, S::Submitted, S::Unknown] {
            assert_eq!(remote_refund_status(&pending), RemoteRefundStatus::Pending);
        }
        assert_eq!(
            remote_refund_status(&S::Paid),
            RemoteRefundStatus::Succeeded
        );
        assert_eq!(
            remote_refund_status(&S::Cancelled),
            RemoteRefundStatus::Canceled
        );
        for returned in [S::Bounced, S::FundsReturned] {
            assert!(matches!(
                remote_refund_status(&returned),
                RemoteRefundStatus::Failed { .. }
            ));
        }
    }

    #[test]
    fn a_freshly_created_refund_is_pending() {
        let refund: gocardless_client::refunds::Refund =
            serde_json::from_value(serde_json::json!({
                "id": "RF_1", "amount": 500, "currency": "EUR", "reference": null,
                "status": "pending_submission", "links": {"payment": "PM_1"}
            }))
            .unwrap();
        match refund_outcome(refund) {
            RefundOutcome::Pending(ack) => assert_eq!(ack.external_refund_id, "RF_1"),
            other => panic!("expected Pending, got {other:?}"),
        }
    }
}
