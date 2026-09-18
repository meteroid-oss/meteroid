//! Mollie connector. A hosted `sequenceType: first` payment creates a mandate; renewals are
//! `recurring` payments on it. Webhooks are unsigned `id=tr_…` notifications, authenticated by
//! a per-connector URL token and resolved by re-reading the payment.

use super::connector::{
    ConnectorCapabilities, ConnectorIdentity, CredentialOps, CustomerOps, HostedSetupCompletion,
    MandateOps, MandateSetupMode, PaymentOps, REQUEST_QUERY_HEADER, ReconcileOps, RefundOps,
    WebhookOps,
};
use super::error::{
    ConnectorError, CustomerFacingMessage, HostedSetupFailed, HostedSetupNotCompleted,
    HostedSetupPending,
};
use super::events::{
    DisputeEvent, NormalizedEventKind, NormalizedEventSubscription, NormalizedWebhookEvent,
    PaymentFailedEvent, PaymentPendingEvent, PaymentRefundedEvent, PaymentSucceededEvent,
};
use super::model::{
    ChargeAcknowledged, ChargeCancelled, ChargeFailure, ChargeOutcome, ChargeReceipt,
    ChargeRequest, CreateCustomerRequest, DeclineKind, ExternalCustomerRef,
    MandateSetupInstruction, MandateSetupRequest, PaymentMethodSnapshot, RefundOutcome,
    RefundRequest, RefundSnapshot, RegisteredWebhook, RemoteTransactionStatus, WebhookDeliveryUnit,
};
use crate::domain::connectors::{
    Connector, MolliePublicData, MollieSensitiveData, ProviderData, ProviderSensitiveData,
};
use crate::domain::enums::ConnectorProviderEnum;
use crate::domain::{Customer, CustomerConnection, PaymentMethodTypeEnum};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use common_domain::ids::BaseId;
use error_stack::Report;
use http::HeaderMap;
use mollie_client::amount::Amount;
use mollie_client::chargebacks::MollieChargeback;
use mollie_client::client::MollieClient;
use mollie_client::customers::CreateCustomer as MollieCreateCustomer;
use mollie_client::error::MollieError;
use mollie_client::mandates::{MandateStatus, MollieMandate};
use mollie_client::payments::{
    CreatePayment, MolliePayment, PaymentStatus, SequenceType, UpdatePayment,
};
use mollie_client::webhook::parse_classic_ping;
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::sync::OnceLock;

pub(super) const MOLLIE_CAPABILITIES: ConnectorCapabilities = ConnectorCapabilities {
    supports_cards: true,
    supports_mandates: true,
    // Refunds are not wired into billing for any provider yet.
    supports_refunds: false,
    supports_partial_refunds: false,
    // 3DS runs on the hosted first payment; renewals are merchant-initiated.
    supports_3ds: true,
    supports_disputes: true,
    // Webhook URL is set per payment; nothing to register.
    supports_self_webhook_registration: false,
    asynchronous_settlement: true,
    supported_payment_methods: &[
        PaymentMethodTypeEnum::Card,
        // Bacs is in beta at Mollie: not offered.
        PaymentMethodTypeEnum::DirectDebitSepa,
    ],
    mandate_setup_mode: MandateSetupMode::HostedRedirect,
    // Notifications have no timestamp; replays are harmless (idempotent re-read).
    webhook_replay_tolerance_secs: 3600,
    // The payment webhook completes a hosted setup even if the return is lost.
    hosted_setup_completion: HostedSetupCompletion::WebhookBacked,
    // A first payment tagged with `meteroid.transaction_id` is the invoice payment: never recharge.
    supports_hosted_invoice_payment: true,
    supports_hosted_checkout: true,
    pending_charge_accepted: true,
};

/// Mollie's `cancelUrl` marker, mirroring GoCardless' `exit_uri`.
pub const ABANDONED_MARKER: &str = "flow_abandoned";

const CARD_METHOD: &str = "creditcard";

/// Bank methods whose `first` payment yields a SEPA `directdebit` mandate.
const SEPA_FIRST_PAYMENT_METHODS: &[&str] =
    &["ideal", "bancontact", "belfius", "eps", "kbc", "paybybank"];
const SEPA_CURRENCY: &str = "EUR";
/// Mollie rejects 0-amount bank payments, so SEPA setup is a €0.01 verification payment
/// (kept by the merchant, not recorded locally).
const SEPA_SETUP_AMOUNT_MINOR: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupRail {
    Card,
    Sepa,
}

/// Card wins when several rails are requested.
fn setup_rail(methods: &[PaymentMethodTypeEnum]) -> Result<SetupRail, String> {
    if methods.contains(&PaymentMethodTypeEnum::Card) {
        Ok(SetupRail::Card)
    } else if methods.contains(&PaymentMethodTypeEnum::DirectDebitSepa) {
        Ok(SetupRail::Sepa)
    } else {
        Err(format!(
            "Mollie supports card and SEPA direct debit only; requested {methods:?}"
        ))
    }
}

fn rail_currency(rail: SetupRail) -> Option<&'static str> {
    match rail {
        SetupRail::Card => None,
        SetupRail::Sepa => Some(SEPA_CURRENCY),
    }
}

/// The API key prefix selects test/live, so one client serves all tenants.
#[derive(Debug, Clone, Copy)]
pub struct MollieConnector;

impl MollieConnector {
    pub fn new() -> Self {
        MollieConnector
    }

    fn client() -> &'static MollieClient {
        static CLIENT: OnceLock<MollieClient> = OnceLock::new();
        CLIENT.get_or_init(MollieClient::new)
    }
}

impl Default for MollieConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectorIdentity for MollieConnector {
    fn provider(&self) -> ConnectorProviderEnum {
        ConnectorProviderEnum::Mollie
    }
    fn capabilities(&self) -> &ConnectorCapabilities {
        &MOLLIE_CAPABILITIES
    }
}

#[async_trait]
impl CredentialOps for MollieConnector {
    /// `GET /v2/methods`: malformed key → 400, unknown → 401, no access → 403. Network errors and
    /// 5xx say nothing about the key.
    async fn validate_credentials(
        &self,
        connector: &Connector,
    ) -> Result<ProviderData, Report<ConnectorError>> {
        let api_key = extract_api_key(connector)?;
        Self::client()
            .list_methods(&api_key)
            .await
            .map_err(|err| {
                match &err {
                MollieError::Mollie(req) if matches!(req.status, 400 | 401 | 403) => {
                    Report::new(ConnectorError::Configuration(
                        "Mollie rejected this API key. Check that it is a valid test_ or live_ key."
                            .to_string(),
                    ))
                }
                _ => Report::new(ConnectorError::Transport(format!(
                    "Couldn't reach Mollie to verify the key: {err}"
                ))),
            }
            })?;
        Ok(ProviderData::Mollie(MolliePublicData::default()))
    }
}

#[async_trait]
impl CustomerOps for MollieConnector {
    async fn create_customer(
        &self,
        connector: &Connector,
        customer: &Customer,
        request: CreateCustomerRequest,
    ) -> Result<ExternalCustomerRef, Report<ConnectorError>> {
        let api_key = extract_api_key(connector)?;

        let metadata = HashMap::from([
            ("meteroid.customer_id".to_string(), customer.id.as_base62()),
            (
                "meteroid.tenant_id".to_string(),
                customer.tenant_id.as_base62(),
            ),
        ]);

        let created = Self::client()
            .create_customer(
                MollieCreateCustomer {
                    name: Some(customer.name.clone()),
                    email: customer.billing_email.clone(),
                    locale: None,
                    metadata: Some(metadata),
                },
                &api_key,
                request.idempotency_key.as_str(),
            )
            .await
            .map_err(|e| map_mollie_error(MollieOp::Customer, e))?;

        Ok(ExternalCustomerRef {
            external_id: created.id,
            provider_request_id: None,
        })
    }
}

#[async_trait]
impl MandateOps for MollieConnector {
    /// Puts the payment id in `redirectUrl` so the return handler can classify the outcome without
    /// a webhook. A duplicate on retry is harmless: only one checkout URL is shown.
    async fn initiate_mandate_setup(
        &self,
        connector: &Connector,
        connection: &CustomerConnection,
        request: MandateSetupRequest<'_>,
    ) -> Result<MandateSetupInstruction, Report<ConnectorError>> {
        let api_key = extract_api_key(connector)?;
        let client = Self::client();

        let rail = setup_rail(request.payment_methods)
            .map_err(|msg| Report::new(ConnectorError::MandateSetup(msg)))?;

        let return_url = request.return_url.clone().ok_or_else(|| {
            Report::new(ConnectorError::MandateSetup(
                "Mollie mandate setup requires a return_url (hosted redirect is the only flow)"
                    .to_string(),
            ))
        })?;

        // Hosted checkout/invoice collects the real amount; a plain setup is €0 (card) or €0.01
        // (SEPA).
        let amount = match hosted_capture(&request) {
            Some((amount_minor, _)) if amount_minor <= 0 => {
                return Err(Report::new(ConnectorError::MandateSetup(format!(
                    "Mollie hosted payment requires a positive amount, got {amount_minor}"
                ))));
            }
            Some((amount_minor, currency)) => {
                if let Some(required) = rail_currency(rail)
                    && !currency.eq_ignore_ascii_case(required)
                {
                    return Err(Report::new(ConnectorError::MandateSetup(format!(
                        "Mollie {rail:?} direct debit collects {required} only; payment is in {currency}"
                    ))));
                }
                amount_from_minor(amount_minor, currency)?
            }
            None => match rail {
                SetupRail::Card => {
                    let currency = request.currency.as_deref().ok_or_else(|| {
                        Report::new(ConnectorError::MandateSetup(
                            "no currency available for the Mollie setup payment".to_string(),
                        ))
                    })?;
                    amount_from_minor(0, currency)?
                }
                SetupRail::Sepa => amount_from_minor(SEPA_SETUP_AMOUNT_MINOR, SEPA_CURRENCY)?,
            },
        };
        let methods: Vec<String> = match rail {
            SetupRail::Card => vec![CARD_METHOD.to_string()],
            SetupRail::Sepa => SEPA_FIRST_PAYMENT_METHODS
                .iter()
                .map(|m| m.to_string())
                .collect(),
        };
        let purpose = if request.invoice_payment.is_some() {
            invoice_purpose(request.descriptor.as_ref())
        } else if request.checkout.is_some() {
            "Subscription".to_string()
        } else if rail == SetupRail::Card {
            "Card setup".to_string()
        } else {
            "Direct debit mandate".to_string()
        };
        let description = payment_description(request.descriptor.as_ref(), &purpose);
        let webhook_url = required_webhook_url(connector, request.webhook_url.as_deref())?;

        let metadata = setup_payment_metadata(connector, connection, &request);
        let cancel_url = with_query_param(&return_url, "error", ABANDONED_MARKER);

        let payment = client
            .create_payment(
                CreatePayment {
                    amount,
                    description,
                    redirect_url: Some(return_url.clone()),
                    cancel_url: Some(cancel_url.clone()),
                    webhook_url: Some(webhook_url),
                    method: Some(methods.clone()),
                    sequence_type: Some(SequenceType::First),
                    customer_id: Some(connection.external_customer_id.clone()),
                    mandate_id: None,
                    locale: None,
                    metadata: Some(metadata),
                },
                &api_key,
                request.idempotency_key.as_str(),
            )
            .await
            .map_err(|e| map_mollie_error(MollieOp::Mandate, e))?;

        // If no requested method can create a mandate, Mollie silently falls back to another one
        // (seen: `creditcard`). Never send a direct-debit customer to a card checkout.
        if !method_within_request(payment.method.as_deref(), &methods) {
            let _ = client.cancel_payment(&payment.id, &api_key).await;
            log::error!(
                "Mollie connector {}: offered `{}` instead of the requested {methods:?} for a \
                 {rail:?} mandate; activate {} on the Mollie profile",
                connector.id,
                payment.method.as_deref().unwrap_or_default(),
                match rail {
                    SetupRail::Card => "Cards",
                    SetupRail::Sepa => "SEPA Direct Debit and a bank method (iDEAL, Bancontact…)",
                }
            );
            let message = match rail {
                SetupRail::Card => "Card payments are not available right now.",
                _ => "Direct debit is not available right now. Please pay by card.",
            };
            return Err(
                Report::new(ConnectorError::Configuration(message.to_string()))
                    .attach_opaque(CustomerFacingMessage(message.to_string())),
            );
        }

        let updated = client
            .update_payment(
                &payment.id,
                UpdatePayment {
                    redirect_url: Some(with_query_param(&return_url, "intent", &payment.id)),
                    cancel_url: Some(with_query_param(&cancel_url, "intent", &payment.id)),
                    webhook_url: None,
                    metadata: None,
                },
                &api_key,
            )
            .await
            .map_err(|e| map_mollie_error(MollieOp::Mandate, e))?;

        let authorisation_url = updated
            .checkout_url()
            .or(payment.checkout_url())
            .ok_or_else(|| {
                Report::new(ConnectorError::MandateSetup(format!(
                    "Mollie payment {} has no checkout link",
                    payment.id
                )))
            })?
            .to_string();

        Ok(MandateSetupInstruction::HostedRedirect {
            intent_id: payment.id,
            authorisation_url,
            expires_at: updated.expires_at.as_deref().and_then(parse_datetime),
        })
    }

    async fn fetch_payment_method(
        &self,
        connector: &Connector,
        external_payment_method_id: &str,
        external_customer_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        let api_key = extract_api_key(connector)?;
        let mandate = Self::client()
            .get_mandate(external_customer_id, external_payment_method_id, &api_key)
            .await
            .map_err(|e| map_mollie_error(MollieOp::PaymentMethod, e))?;
        // Mandates carry no metadata — meteroid_* fields stay None.
        Ok(snapshot_from_mandate(mandate, &HashMap::new(), None))
    }

    /// Incomplete payment → retryable ([`HostedSetupPending`]); dead payment → terminal.
    async fn complete_mandate_setup(
        &self,
        connector: &Connector,
        intent_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        let api_key = extract_api_key(connector)?;
        let client = Self::client();

        let payment = client
            .get_payment(intent_id, &api_key)
            .await
            .map_err(|e| map_mollie_error(MollieOp::Mandate, e))?;

        let (customer_id, mandate_id) = match setup_state(&payment) {
            SetupState::Completed => (
                payment.customer_id.clone().unwrap_or_default(),
                payment.mandate_id.clone().unwrap_or_default(),
            ),
            SetupState::Pending => return Err(pending_setup_error(intent_id, &payment.status)),
            SetupState::Failed => {
                return Err(Report::new(ConnectorError::MandateSetup(format!(
                    "Mollie payment {intent_id} ended {:?} without a mandate",
                    payment.status
                )))
                .attach_opaque(HostedSetupFailed));
            }
        };

        let (customer_ref, mandate_ref, key) =
            (customer_id.as_str(), mandate_id.as_str(), &api_key);
        let mandate = await_valid_mandate(&mandate_id, &MANDATE_VALIDITY_BACKOFF, || async move {
            client
                .get_mandate(customer_ref, mandate_ref, key)
                .await
                .map_err(|e| map_mollie_error(MollieOp::Mandate, e))
        })
        .await?;

        // A first payment created for a transaction is that payment: recorded, never recharged.
        let captured = payment
            .metadata
            .contains_key("meteroid.transaction_id")
            .then(|| payment.id.clone());
        Ok(snapshot_from_mandate(mandate, &payment.metadata, captured))
    }

    /// Errors if the payment could still capture and can't be cancelled, so the caller completes it
    /// instead of creating a second payment.
    async fn cancel_mandate_setup(
        &self,
        connector: &Connector,
        intent_id: &str,
    ) -> Result<(), Report<ConnectorError>> {
        let api_key = extract_api_key(connector)?;
        let client = Self::client();
        let payment = match client.get_payment(intent_id, &api_key).await {
            Ok(payment) => payment,
            // Never created at Mollie: nothing can capture.
            Err(MollieError::Mollie(req_err)) if req_err.status == 404 => return Ok(()),
            Err(e) => return Err(map_mollie_error(MollieOp::Mandate, e)),
        };
        match cancel_decision(&payment) {
            CancelDecision::AlreadyDead => Ok(()),
            CancelDecision::Cancel => client
                .cancel_payment(intent_id, &api_key)
                .await
                .map(|_| ())
                .map_err(|e| map_mollie_error(MollieOp::Mandate, e)),
            CancelDecision::Adopt => Err(Report::new(ConnectorError::MandateSetup(format!(
                "Mollie payment {intent_id} is {:?} and not cancelable",
                payment.status
            )))),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CancelDecision {
    AlreadyDead,
    Cancel,
    Adopt,
}

fn cancel_decision(payment: &MolliePayment) -> CancelDecision {
    match payment.status {
        PaymentStatus::Failed | PaymentStatus::Canceled | PaymentStatus::Expired => {
            CancelDecision::AlreadyDead
        }
        PaymentStatus::Open | PaymentStatus::Pending if payment.is_cancelable == Some(true) => {
            CancelDecision::Cancel
        }
        // Paid, authorized, open but not cancelable, or unknown status.
        _ => CancelDecision::Adopt,
    }
}

#[async_trait]
impl PaymentOps for MollieConnector {
    /// Built from committed state: a retry of an accepted attempt reuses the key, an attempt after
    /// a decline gets a new one.
    fn invoice_charge_idempotency_seed(
        &self,
        payment_method_id: &common_domain::ids::CustomerPaymentMethodId,
        invoice_id: &common_domain::ids::InvoiceId,
        prior_invoice_attempts: usize,
    ) -> Option<String> {
        Some(format!(
            "mollie-charge:{}:{}:{prior_invoice_attempts}",
            payment_method_id.as_base62(),
            invoice_id.as_base62(),
        ))
    }

    /// Mollie keeps idempotency keys for one hour; after that, the local pending and overpayment
    /// checks prevent double charges.
    async fn charge_off_session(
        &self,
        connector: &Connector,
        request: ChargeRequest<'_>,
    ) -> Result<ChargeOutcome, Report<ConnectorError>> {
        let api_key = extract_api_key(connector)?;

        // Unacceptable charges fail the attempt (dunning) instead of retrying into the dead-letter
        // queue.
        let invalid = if request.amount_minor <= 0 {
            Some(format!(
                "Mollie charge requires a positive amount, got {}",
                request.amount_minor
            ))
        } else {
            validate_charge_rail(&request.payment_method_type, request.currency).err()
        };
        if let Some(message) = invalid {
            return Ok(ChargeOutcome::Failed(ChargeFailure {
                external_id: None,
                code: Some("invalid_charge".to_string()),
                message,
                retryable: false,
                decline_kind: DeclineKind::Other,
                provider_request_id: None,
            }));
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

        let amount = amount_from_minor(request.amount_minor, request.currency)?;
        let webhook_url = required_webhook_url(connector, request.webhook_url.as_deref())?;
        let client = Self::client();
        let created = client
            .create_payment(
                CreatePayment {
                    amount,
                    description: payment_description(
                        request.descriptor.as_ref(),
                        &invoice_purpose(request.descriptor.as_ref()),
                    ),
                    redirect_url: None,
                    cancel_url: None,
                    webhook_url: Some(webhook_url),
                    method: None,
                    sequence_type: Some(SequenceType::Recurring),
                    customer_id: Some(request.customer_external_id.to_string()),
                    mandate_id: Some(request.payment_method_external_id.to_string()),
                    locale: None,
                    metadata: Some(metadata),
                },
                &api_key,
                request.idempotency_key.as_str(),
            )
            .await;
        let payment = match created {
            Ok(payment) => payment,
            // A refusal is a terminal decline for dunning, not a retryable error.
            Err(MollieError::Mollie(req_err)) if is_rejected_charge(req_err.status) => {
                let mandate_revoked = match client
                    .get_mandate(
                        request.customer_external_id,
                        request.payment_method_external_id,
                        &api_key,
                    )
                    .await
                {
                    Ok(mandate) => mandate.status == MandateStatus::Invalid,
                    Err(MollieError::Mollie(e)) => matches!(e.status, 404 | 410),
                    Err(_) => false,
                };
                return Ok(ChargeOutcome::Failed(rejected_charge(
                    &req_err,
                    mandate_revoked,
                )));
            }
            Err(e) => return Err(map_mollie_error(MollieOp::Charge, e)),
        };

        payment_to_outcome(&payment)
    }
}

#[async_trait]
impl RefundOps for MollieConnector {
    async fn refund(
        &self,
        _connector: &Connector,
        _request: RefundRequest<'_>,
    ) -> Result<RefundOutcome, Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Mollie,
            capability: "refund",
        }))
    }

    /// Refunds are read from the payment's `amountRefunded`; unused.
    async fn fetch_refund(
        &self,
        _connector: &Connector,
        _external_refund_id: &str,
    ) -> Result<RefundSnapshot, Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Mollie,
            capability: "fetch_refund",
        }))
    }
}

#[async_trait]
impl ReconcileOps for MollieConnector {
    async fn fetch_transaction_status(
        &self,
        connector: &Connector,
        external_transaction_id: &str,
    ) -> Result<RemoteTransactionStatus, Report<ConnectorError>> {
        let api_key = extract_api_key(connector)?;

        match Self::client()
            .get_payment(external_transaction_id, &api_key)
            .await
        {
            Ok(payment) => remote_status_from_payment(&payment),
            Err(MollieError::Mollie(req_err)) if req_err.status == 404 => {
                Ok(RemoteTransactionStatus::Unknown)
            }
            Err(e) => Err(map_mollie_error(MollieOp::Charge, e)),
        }
    }
}

#[async_trait]
impl WebhookOps for MollieConnector {
    async fn register_webhook(
        &self,
        _connector: &Connector,
        _url: &str,
        _subscriptions: &[NormalizedEventSubscription],
    ) -> Result<RegisteredWebhook, Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Mollie,
            capability: "webhook.register (webhooks are per-payment)",
        }))
    }

    async fn unregister_webhook(
        &self,
        _connector: &Connector,
        _endpoint_id: &str,
    ) -> Result<(), Report<ConnectorError>> {
        Err(Report::new(ConnectorError::Unsupported {
            provider: ConnectorProviderEnum::Mollie,
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
            provider: ConnectorProviderEnum::Mollie,
            capability: "webhook.sync",
        }))
    }

    /// Notifications are `id=tr_…` form posts with no event id; one is sent per status change.
    fn split_delivery(
        &self,
        _connector: &Connector,
        payload: &[u8],
    ) -> Result<Vec<WebhookDeliveryUnit>, Report<ConnectorError>> {
        Ok(vec![WebhookDeliveryUnit {
            event_id: None,
            body: payload.to_vec(),
        }])
    }

    /// Notifications are unsigned: they authenticate with the per-connector `token` query param,
    /// passed on by the router via [`REQUEST_QUERY_HEADER`].
    fn verify_signature(
        &self,
        _connector: &Connector,
        _payload: &[u8],
        headers: &HeaderMap,
        secret: &SecretString,
    ) -> Result<(), Report<ConnectorError>> {
        let query = headers
            .get(REQUEST_QUERY_HEADER)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| Report::new(ConnectorError::SignatureMissing))?;
        let presented = query_param(query, "token")
            .ok_or_else(|| Report::new(ConnectorError::SignatureMissing))?;
        if constant_time_eq(presented.as_bytes(), secret.expose_secret().as_bytes()) {
            Ok(())
        } else {
            Err(Report::new(ConnectorError::SignatureVerification))
        }
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

    /// Only the payment id is read from the request; state comes from `resolve_resource_change`.
    fn parse_events(
        &self,
        _connector: &Connector,
        payload: &[u8],
        _headers: &HeaderMap,
    ) -> Result<Vec<NormalizedWebhookEvent>, Report<ConnectorError>> {
        let payment_id = parse_classic_ping(payload).ok_or_else(|| {
            Report::new(ConnectorError::PayloadDecode(
                "mollie webhook body is not an `id=` ping".to_string(),
            ))
        })?;
        Ok(vec![NormalizedWebhookEvent {
            provider_event_id: format!("ping:{payment_id}"),
            provider_event_type: "payment.changed".to_string(),
            occurred_at: Utc::now(),
            kind: NormalizedEventKind::ResourceChanged {
                resource_ref: payment_id,
            },
            owner_tenant_id: None,
        }])
    }

    /// Unknown ids are acked, not retried: a payment we can't read isn't ours.
    async fn resolve_resource_change(
        &self,
        connector: &Connector,
        resource_ref: &str,
    ) -> Result<Vec<NormalizedWebhookEvent>, Report<ConnectorError>> {
        let api_key = extract_api_key(connector)?;
        let client = Self::client();

        let payment = match client.get_payment(resource_ref, &api_key).await {
            Ok(payment) => payment,
            Err(MollieError::Mollie(req_err)) if req_err.status == 404 => {
                log::warn!("Mollie webhook named unknown payment {resource_ref}; acknowledging");
                return Ok(vec![acknowledged(resource_ref, "mollie payment not found")]);
            }
            Err(e) => return Err(map_mollie_error(MollieOp::Charge, e)),
        };

        // `amountChargedBack` disappears once reversed, so also check `_links.chargebacks`.
        let has_chargebacks = payment.amount_charged_back.is_some()
            || payment
                .links
                .as_ref()
                .is_some_and(|l| l.chargebacks.is_some());
        let chargebacks = if has_chargebacks {
            client
                .list_payment_chargebacks(&payment.id, &api_key)
                .await
                .map_err(|e| map_mollie_error(MollieOp::Charge, e))?
                .into_chargebacks()
        } else {
            Vec::new()
        };

        let mut events = events_for_payment(&payment, &chargebacks)?;

        // Mollie has no mandate webhook: after a failed renewal, re-read the mandate and detach it
        // if revoked, otherwise every later cycle fails on it.
        if needs_mandate_recheck(&payment)
            && let (Some(customer), Some(mandate_id)) = (
                payment.customer_id.as_deref(),
                payment.mandate_id.as_deref(),
            )
        {
            let invalid = match client.get_mandate(customer, mandate_id, &api_key).await {
                Ok(mandate) => mandate.status == MandateStatus::Invalid,
                Err(MollieError::Mollie(e)) if matches!(e.status, 404 | 410) => true,
                Err(e) => return Err(map_mollie_error(MollieOp::Mandate, e)),
            };
            if invalid {
                events.push(mandate_detached_event(&payment, mandate_id));
            }
        }
        Ok(events)
    }
}

fn needs_mandate_recheck(payment: &MolliePayment) -> bool {
    payment.sequence_type == Some(SequenceType::Recurring)
        && payment.status == PaymentStatus::Failed
}

fn mandate_detached_event(payment: &MolliePayment, mandate_id: &str) -> NormalizedWebhookEvent {
    NormalizedWebhookEvent {
        provider_event_id: format!("{}:mandate-invalid", payment.id),
        provider_event_type: "mandate.invalid".to_string(),
        occurred_at: Utc::now(),
        kind: NormalizedEventKind::PaymentMethodDetached(
            super::events::PaymentMethodDetachedEvent {
                external_payment_method_id: mandate_id.to_string(),
                reason: Some("mandate invalid at Mollie after a failed charge".to_string()),
            },
        ),
        owner_tenant_id: payment.metadata.get("meteroid.tenant_id").cloned(),
    }
}

fn extract_sensitive(
    connector: &Connector,
) -> Result<&MollieSensitiveData, Report<ConnectorError>> {
    match &connector.sensitive {
        Some(ProviderSensitiveData::Mollie(d)) => Ok(d),
        Some(_) => Err(Report::new(ConnectorError::Configuration(
            "connector is not a mollie connector".into(),
        ))),
        None => Err(Report::new(ConnectorError::Configuration(
            "mollie connector has no api_key".into(),
        ))),
    }
}

fn extract_api_key(connector: &Connector) -> Result<SecretString, Report<ConnectorError>> {
    Ok(SecretString::from(
        extract_sensitive(connector)?.api_key.clone(),
    ))
}

/// Inbound webhook endpoint plus the per-connector auth token.
pub fn webhook_url(connector: &Connector, endpoint: Option<&str>) -> Option<String> {
    let token = match &connector.sensitive {
        Some(ProviderSensitiveData::Mollie(d)) => d.webhook_token.clone(),
        _ => return None,
    };
    Some(with_query_param(endpoint?, "token", &token))
}

/// SEPA mandates collect EUR only; Bacs (beta) and ACH are not offered.
fn validate_charge_rail(method: &PaymentMethodTypeEnum, currency: &str) -> Result<(), String> {
    let required = match method {
        PaymentMethodTypeEnum::DirectDebitSepa => SEPA_CURRENCY,
        PaymentMethodTypeEnum::DirectDebitBacs | PaymentMethodTypeEnum::DirectDebitAch => {
            return Err(format!(
                "Mollie {method:?} is not supported; only card and SEPA are"
            ));
        }
        _ => return Ok(()),
    };
    if currency.eq_ignore_ascii_case(required) {
        Ok(())
    } else {
        Err(format!(
            "Mollie {method:?} mandate is {required}-only; cannot charge {currency}"
        ))
    }
}

/// Without a webhook, a hosted checkout capture could never settle.
fn required_webhook_url(
    connector: &Connector,
    endpoint: Option<&str>,
) -> Result<String, Report<ConnectorError>> {
    webhook_url(connector, endpoint).ok_or_else(|| {
        Report::new(ConnectorError::Configuration(
            "no webhook_url for the mollie payment; it would have no settlement callback".into(),
        ))
    })
}

/// Minor-unit exponent from the ISO 4217 table.
fn currency_exponent(currency: &str) -> Result<u32, Report<ConnectorError>> {
    rusty_money::iso::find(&currency.to_ascii_uppercase())
        .map(|c| c.exponent)
        .ok_or_else(|| {
            Report::new(ConnectorError::Configuration(format!(
                "unknown currency {currency} for a Mollie amount"
            )))
        })
}

fn amount_from_minor(minor: i64, currency: &str) -> Result<Amount, Report<ConnectorError>> {
    Ok(Amount::from_minor(
        minor,
        currency,
        currency_exponent(currency)?,
    ))
}

/// Unparseable is an error, not `0`: a zero would leave a captured payment unsettled.
fn amount_to_minor(payment: &MolliePayment) -> Result<i64, Report<ConnectorError>> {
    parse_amount(&payment.amount, &payment.id)
}

fn parse_amount(amount: &Amount, owner: &str) -> Result<i64, Report<ConnectorError>> {
    amount
        .to_minor(currency_exponent(&amount.currency)?)
        .map_err(|e| {
            Report::new(ConnectorError::Configuration(format!(
                "mollie {owner} has an unparseable amount: {e}"
            )))
        })
}

fn with_query_param(url: &str, key: &str, value: &str) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}{key}={}", urlencoding::encode(value))
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| {
            urlencoding::decode(v)
                .map(|c| c.into_owned())
                .unwrap_or_default()
        })
    })
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn parse_datetime(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// The amount a hosted checkout or in-flow invoice payment collects.
fn hosted_capture<'a>(request: &'a MandateSetupRequest<'_>) -> Option<(i64, &'a str)> {
    request
        .checkout
        .as_ref()
        .map(|c| (c.amount_minor, c.currency.as_str()))
        .or_else(|| {
            request
                .invoice_payment
                .as_ref()
                .map(|i| (i.amount_minor, i.currency.as_str()))
        })
}

fn setup_payment_metadata(
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

/// Mandate `details` → snapshot. `cardExpiryDate` is `YYYY-MM-DD` on mandates.
fn snapshot_from_mandate(
    mandate: MollieMandate,
    metadata: &HashMap<String, String>,
    payment_request_payment: Option<String>,
) -> PaymentMethodSnapshot {
    let payment_method_type = match mandate.method.as_deref() {
        Some("creditcard") => PaymentMethodTypeEnum::Card,
        Some("directdebit") => PaymentMethodTypeEnum::DirectDebitSepa,
        other => {
            log::warn!(
                "Mollie mandate {} has unmapped method {other:?}; storing as Other",
                mandate.id
            );
            PaymentMethodTypeEnum::Other
        }
    };
    let details = mandate.details.unwrap_or_default();
    let (exp_year, exp_month) = details
        .card_expiry_date
        .as_deref()
        .and_then(parse_expiry)
        .map_or((None, None), |(y, m)| (Some(y), Some(m)));
    let iban = details.consumer_account.as_deref().map(normalize_iban);
    let account_number_hint = iban
        .as_deref()
        .filter(|iban| iban.len() >= 4)
        .map(|iban| iban[iban.len() - 4..].to_string());
    let fingerprint = match payment_method_type {
        PaymentMethodTypeEnum::Card => details.card_fingerprint,
        PaymentMethodTypeEnum::DirectDebitSepa => iban.as_deref().map(iban_fingerprint),
        _ => None,
    };
    PaymentMethodSnapshot {
        external_payment_method_id: mandate.id,
        payment_method_type,
        account_number_hint,
        card_brand: details.card_label,
        card_last4: details.card_number,
        card_exp_month: exp_month,
        card_exp_year: exp_year,
        fingerprint,
        meteroid_connection_id: metadata.get("meteroid.connection_id").cloned(),
        meteroid_customer_id: metadata.get("meteroid.customer_id").cloned(),
        meteroid_invoice_id: metadata.get("meteroid.invoice_id").cloned(),
        meteroid_checkout_session_id: metadata.get("meteroid.checkout_session_id").cloned(),
        meteroid_transaction_id: metadata.get("meteroid.transaction_id").cloned(),
        payment_request_payment,
    }
}

fn normalize_iban(iban: &str) -> String {
    iban.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// SEPA mandates have the IBAN but no fingerprint: hash it so re-adds dedupe without storing it.
fn iban_fingerprint(iban: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("iban:{}", hex::encode(Sha256::digest(iban.as_bytes())))
}

/// `YYYY-MM-DD` (mandates) or `MM/YY` (payments) → (year, month).
fn parse_expiry(raw: &str) -> Option<(i32, i32)> {
    if let Some((y, rest)) = raw.split_once('-') {
        let (m, _) = rest.split_once('-')?;
        return Some((y.parse().ok()?, m.parse().ok()?));
    }
    let (m, y) = raw.split_once('/')?;
    let year: i32 = y.parse().ok()?;
    Some((
        if y.len() == 2 { 2000 + year } else { year },
        m.parse().ok()?,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupState {
    Completed,
    Pending,
    Failed,
}

/// A mandate can still be `pending` right after its first payment is `paid`, and no webhook fires
/// when it becomes valid, so re-read it briefly instead of going back to the queue.
const MANDATE_VALIDITY_BACKOFF: [std::time::Duration; 5] = [
    std::time::Duration::from_millis(250),
    std::time::Duration::from_millis(500),
    std::time::Duration::from_secs(1),
    std::time::Duration::from_secs(2),
    std::time::Duration::from_secs(4),
];

/// Still not valid after the backoff → retryable ([`HostedSetupPending`]); never charge it.
async fn await_valid_mandate<F, Fut>(
    mandate_id: &str,
    backoff: &[std::time::Duration],
    mut fetch: F,
) -> Result<MollieMandate, Report<ConnectorError>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<MollieMandate, Report<ConnectorError>>>,
{
    let mut delays = backoff.iter();
    loop {
        let mandate = fetch().await?;
        match mandate.status {
            MandateStatus::Valid => return Ok(mandate),
            MandateStatus::Invalid => {
                return Err(Report::new(ConnectorError::MandateSetup(format!(
                    "Mollie mandate {mandate_id} is invalid"
                )))
                .attach_opaque(HostedSetupFailed));
            }
            MandateStatus::Pending | MandateStatus::Unknown => match delays.next() {
                Some(delay) => tokio::time::sleep(*delay).await,
                None => {
                    return Err(Report::new(ConnectorError::MandateSetup(format!(
                        "Mollie mandate {mandate_id} is not valid yet ({:?})",
                        mandate.status
                    )))
                    .attach_opaque(HostedSetupPending));
                }
            },
        }
    }
}

/// `open` = the customer never finished the hosted page ([`HostedSetupNotCompleted`]).
fn pending_setup_error(intent_id: &str, status: &PaymentStatus) -> Report<ConnectorError> {
    let report = Report::new(ConnectorError::MandateSetup(format!(
        "Mollie payment {intent_id} has no mandate yet (status {status:?})"
    )))
    .attach_opaque(HostedSetupPending);
    if *status == PaymentStatus::Open {
        report.attach_opaque(HostedSetupNotCompleted)
    } else {
        report
    }
}

/// Complete only when `paid` with a mandate (`authorized` can still expire); otherwise pending
/// unless dead.
fn setup_state(payment: &MolliePayment) -> SetupState {
    let has_mandate = payment.mandate_id.as_deref().is_some_and(|m| !m.is_empty())
        && payment
            .customer_id
            .as_deref()
            .is_some_and(|c| !c.is_empty());
    match payment.status {
        PaymentStatus::Paid if has_mandate => SetupState::Completed,
        PaymentStatus::Failed | PaymentStatus::Canceled | PaymentStatus::Expired => {
            SetupState::Failed
        }
        _ => SetupState::Pending,
    }
}

fn decline_kind_for(failure_reason: Option<&str>) -> DeclineKind {
    match failure_reason {
        Some("insufficient_funds") => DeclineKind::InsufficientFunds,
        Some("card_expired" | "inactive_card") => DeclineKind::CardExpired,
        Some(
            "refused_by_issuer"
            | "card_declined"
            | "invalid_card_number"
            | "invalid_cvv"
            | "invalid_card_holder_name"
            | "invalid_card_type",
        ) => DeclineKind::DoNotHonor,
        Some("possible_fraud") => DeclineKind::Fraud,
        Some(r) if r.starts_with("authentication_") => DeclineKind::AuthenticationRequired,
        _ => DeclineKind::Other,
    }
}

fn failure_code(payment: &MolliePayment) -> Option<String> {
    payment.details.as_ref().and_then(|d| {
        d.failure_reason
            .clone()
            .or_else(|| d.bank_reason_code.clone())
    })
}

fn failure_message(payment: &MolliePayment) -> String {
    payment
        .details
        .as_ref()
        .and_then(|d| d.failure_message.clone().or_else(|| d.bank_reason.clone()))
        .unwrap_or_else(|| format!("Mollie payment {:?}", payment.status))
}

fn processed_at(payment: &MolliePayment) -> NaiveDateTime {
    payment
        .paid_at
        .as_deref()
        .and_then(parse_datetime)
        .map(|dt| dt.naive_utc())
        .unwrap_or_else(|| Utc::now().naive_utc())
}

fn payment_to_outcome(payment: &MolliePayment) -> Result<ChargeOutcome, Report<ConnectorError>> {
    let id = payment.id.clone();
    Ok(match payment.status {
        PaymentStatus::Paid => ChargeOutcome::Succeeded(ChargeReceipt {
            external_id: id,
            amount_received_minor: amount_to_minor(payment)?,
            processed_at: processed_at(payment),
            provider_request_id: None,
        }),
        // `Unknown` status: don't guess; the webhook or reconciliation re-reads it.
        PaymentStatus::Open
        | PaymentStatus::Pending
        | PaymentStatus::Authorized
        | PaymentStatus::Unknown => ChargeOutcome::Pending(ChargeAcknowledged {
            external_id: id,
            provider_request_id: None,
        }),
        PaymentStatus::Canceled => ChargeOutcome::Cancelled(ChargeCancelled {
            external_id: Some(id),
            message: "Payment cancelled".to_string(),
            provider_request_id: None,
        }),
        PaymentStatus::Failed | PaymentStatus::Expired => {
            let code = failure_code(payment);
            ChargeOutcome::Failed(ChargeFailure {
                external_id: Some(id),
                decline_kind: decline_kind_for(code.as_deref()),
                code,
                message: failure_message(payment),
                retryable: false,
                provider_request_id: None,
            })
        }
    })
}

fn remote_status_from_payment(
    payment: &MolliePayment,
) -> Result<RemoteTransactionStatus, Report<ConnectorError>> {
    Ok(match payment.status {
        PaymentStatus::Paid => RemoteTransactionStatus::Succeeded {
            amount_received_minor: amount_to_minor(payment)?,
            currency: payment.amount.currency.clone(),
            processed_at: processed_at(payment),
        },
        PaymentStatus::Open
        | PaymentStatus::Pending
        | PaymentStatus::Authorized
        | PaymentStatus::Unknown => RemoteTransactionStatus::Pending,
        PaymentStatus::Canceled => RemoteTransactionStatus::Cancelled,
        PaymentStatus::Failed | PaymentStatus::Expired => {
            let code = failure_code(payment);
            RemoteTransactionStatus::Failed {
                decline_kind: decline_kind_for(code.as_deref()),
                code,
                message: failure_message(payment),
            }
        }
    })
}

fn acknowledged(payment_id: &str, reason: &'static str) -> NormalizedWebhookEvent {
    NormalizedWebhookEvent {
        provider_event_id: format!("{payment_id}:acknowledged"),
        provider_event_type: "payment.changed".to_string(),
        occurred_at: Utc::now(),
        kind: NormalizedEventKind::Acknowledged { reason },
        owner_tenant_id: None,
    }
}

/// Event order: mandate completion (binds the checkout tx), payment state, then reversals oldest
/// first (the reversal store drops anything older than its `reversed_at` high-water mark).
fn events_for_payment(
    payment: &MolliePayment,
    chargebacks: &[MollieChargeback],
) -> Result<Vec<NormalizedWebhookEvent>, Report<ConnectorError>> {
    let owner_tenant_id = payment.metadata.get("meteroid.tenant_id").cloned();
    let meteroid_tx = payment.metadata.get("meteroid.transaction_id").cloned();
    let amount_minor = amount_to_minor(payment)?;
    let is_first = payment.sequence_type == Some(SequenceType::First);
    // Setup payments (€0 card / €0.01 SEPA) name no transaction.
    let is_setup_only = is_first && meteroid_tx.is_none();
    let status_tag = format!("{:?}", payment.status).to_lowercase();

    let event = |suffix: &str, occurred_at: DateTime<Utc>, kind: NormalizedEventKind| {
        NormalizedWebhookEvent {
            provider_event_id: format!("{}:{suffix}", payment.id),
            provider_event_type: format!("payment.{status_tag}"),
            occurred_at,
            kind,
            owner_tenant_id: owner_tenant_id.clone(),
        }
    };
    let at = |raw: &Option<String>| {
        raw.as_deref()
            .and_then(parse_datetime)
            .unwrap_or_else(Utc::now)
    };

    let mut events = Vec::new();

    if is_first {
        match setup_state(payment) {
            SetupState::Completed => events.push(event(
                "mandate",
                at(&payment.paid_at),
                NormalizedEventKind::MandateSetupCompleted {
                    provider_intent_id: payment.id.clone(),
                },
            )),
            SetupState::Pending | SetupState::Failed if is_setup_only => {
                events.push(event(
                    "setup",
                    Utc::now(),
                    NormalizedEventKind::Acknowledged {
                        reason: "mollie setup payment not completed",
                    },
                ));
                return Ok(events);
            }
            _ => {}
        }
        if is_setup_only {
            return Ok(events);
        }
    }

    let state = match payment.status {
        PaymentStatus::Paid => NormalizedEventKind::PaymentSucceeded(PaymentSucceededEvent {
            external_transaction_id: payment.id.clone(),
            amount_received_minor: amount_minor,
            currency: payment.amount.currency.clone(),
            meteroid_transaction_id: meteroid_tx.clone(),
        }),
        PaymentStatus::Failed | PaymentStatus::Expired => {
            NormalizedEventKind::PaymentFailed(PaymentFailedEvent {
                external_transaction_id: payment.id.clone(),
                code: failure_code(payment),
                message: failure_message(payment),
                retryable: false,
                meteroid_transaction_id: meteroid_tx.clone(),
            })
        }
        PaymentStatus::Canceled => NormalizedEventKind::PaymentFailed(PaymentFailedEvent {
            external_transaction_id: payment.id.clone(),
            code: Some("canceled".into()),
            message: "Payment cancelled".into(),
            retryable: false,
            meteroid_transaction_id: meteroid_tx.clone(),
        }),
        PaymentStatus::Open
        | PaymentStatus::Pending
        | PaymentStatus::Authorized
        | PaymentStatus::Unknown => NormalizedEventKind::PaymentPending(PaymentPendingEvent {
            external_transaction_id: payment.id.clone(),
            meteroid_transaction_id: meteroid_tx.clone(),
        }),
    };
    let state_at = match payment.status {
        PaymentStatus::Paid => at(&payment.paid_at),
        PaymentStatus::Failed => at(&payment.failed_at),
        PaymentStatus::Expired => at(&payment.expired_at),
        PaymentStatus::Canceled => at(&payment.canceled_at),
        _ => Utc::now(),
    };
    events.push(event("state", state_at, state));

    let mut reversals = Vec::new();

    // Cumulative and monotonic, so redelivery is idempotent.
    let refunded = match payment.amount_refunded.as_ref() {
        Some(amount) => parse_amount(amount, &format!("{} amountRefunded", payment.id))?,
        None => 0,
    };
    if refunded > 0 {
        reversals.push(event(
            "refunded",
            Utc::now(),
            NormalizedEventKind::PaymentRefunded(PaymentRefundedEvent {
                external_transaction_id: payment.id.clone(),
                external_refund_id: format!("{}:refunds", payment.id),
                amount_refunded_minor: refunded,
                currency: payment.amount.currency.clone(),
            }),
        ));
    }

    for chargeback in chargebacks {
        let amount = parse_amount(&chargeback.amount, &chargeback.id)?;
        let dispute = DisputeEvent {
            external_dispute_id: chargeback.id.clone(),
            external_transaction_id: payment.id.clone(),
            amount_minor: amount.abs(),
            currency: chargeback.amount.currency.clone(),
            reason: chargeback.reason.as_ref().and_then(|r| r.code.clone()),
        };
        match &chargeback.reversed_at {
            None => reversals.push(event(
                &format!("chargeback:{}", chargeback.id),
                at(&chargeback.created_at),
                NormalizedEventKind::DisputeFundsWithdrawn(dispute),
            )),
            Some(reversed_at) => reversals.push(event(
                &format!("chargeback-reversed:{}", chargeback.id),
                at(&Some(reversed_at.clone())),
                NormalizedEventKind::DisputeFundsReinstated(dispute),
            )),
        }
    }

    reversals.sort_by_key(|e| e.occurred_at);
    events.extend(reversals);
    Ok(events)
}

/// Maps a 4xx to the matching `ConnectorError`, not always a failed charge.
#[derive(Clone, Copy, Debug)]
enum MollieOp {
    Customer,
    Mandate,
    PaymentMethod,
    Charge,
}

impl MollieOp {
    fn logical_error(self, msg: String) -> ConnectorError {
        match self {
            MollieOp::Customer => ConnectorError::CustomerOp(msg),
            MollieOp::Mandate => ConnectorError::MandateSetup(msg),
            MollieOp::PaymentMethod => ConnectorError::CustomerOp(msg),
            MollieOp::Charge => ConnectorError::Charge(msg),
        }
    }
}

/// Shown on Mollie's page and usually the bank statement: no internal ids.
fn payment_description(
    descriptor: Option<&super::model::PaymentDescriptor>,
    purpose: &str,
) -> String {
    let text = match descriptor
        .map(|d| d.merchant_name.trim())
        .filter(|m| !m.is_empty())
    {
        Some(merchant) => format!("{merchant} – {purpose}"),
        None => purpose.to_string(),
    };
    // Mollie caps descriptions at 255 characters.
    text.chars().take(255).collect()
}

/// `None` lets the customer choose among the requested methods on Mollie's page.
fn method_within_request(returned: Option<&str>, requested: &[String]) -> bool {
    returned.is_none_or(|m| requested.iter().any(|r| r == m))
}

fn invoice_purpose(descriptor: Option<&super::model::PaymentDescriptor>) -> String {
    match descriptor.and_then(|d| d.invoice_number.as_deref()) {
        Some(number) => format!("Invoice {number}"),
        None => "Payment".to_string(),
    }
}

/// A 4xx that retrying won't fix. 401/403 are configuration errors; 408/409/429 are transient.
fn is_rejected_charge(status: u16) -> bool {
    (400..500).contains(&status) && !matches!(status, 401 | 403 | 408 | 409 | 429)
}

fn rejected_charge(
    req_err: &mollie_client::error::RequestError,
    mandate_revoked: bool,
) -> ChargeFailure {
    ChargeFailure {
        external_id: None,
        code: Some(if mandate_revoked {
            "mandate_invalid".to_string()
        } else {
            format!("mollie_{}", req_err.status)
        }),
        message: format!("Mollie refused the payment ({}): {req_err}", req_err.status),
        retryable: false,
        decline_kind: if mandate_revoked {
            DeclineKind::MandateInactive
        } else {
            DeclineKind::Other
        },
        provider_request_id: None,
    }
}

fn map_mollie_error(op: MollieOp, e: MollieError) -> Report<ConnectorError> {
    match e {
        MollieError::ClientError(msg) => Report::new(ConnectorError::Transport(msg)),
        MollieError::Mollie(req_err) if req_err.status >= 500 => Report::new(
            ConnectorError::Transport(format!("mollie 5xx ({}): {req_err}", req_err.status)),
        ),
        // Configuration, not a terminal setup/charge failure.
        MollieError::Mollie(req_err) if matches!(req_err.status, 401 | 403) => {
            Report::new(ConnectorError::Configuration(format!(
                "mollie rejected credentials ({}): {req_err}",
                req_err.status
            )))
        }
        // Throttled, timed out, or duplicate in flight: retry with the same key.
        MollieError::Mollie(req_err) if matches!(req_err.status, 408 | 409 | 429) => Report::new(
            ConnectorError::Transport(format!("mollie transient ({}): {req_err}", req_err.status)),
        ),
        MollieError::Mollie(req_err) => Report::new(
            op.logical_error(format!("mollie rejected ({}): {req_err}", req_err.status)),
        ),
        MollieError::JSONSerialize(e) => {
            Report::new(ConnectorError::Configuration(format!("mollie: {e}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::connectors::{Connector, MolliePublicData, ProviderData};
    use crate::domain::enums::ConnectorTypeEnum;
    use common_domain::ids::{ConnectorId, TenantId};
    use mollie_client::mandates::MandateDetails;
    use mollie_client::payments::PaymentDetails;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn test_connector() -> Connector {
        Connector {
            id: ConnectorId::new(),
            created_at: NaiveDateTime::default(),
            tenant_id: TenantId::new(),
            alias: "mollie-test".into(),
            connector_type: ConnectorTypeEnum::PaymentProvider,
            provider: ConnectorProviderEnum::Mollie,
            data: Some(ProviderData::Mollie(MolliePublicData {})),
            sensitive: Some(ProviderSensitiveData::Mollie(MollieSensitiveData {
                api_key: "test_unit".into(),
                webhook_token: TOKEN.into(),
            })),
        }
    }

    fn mandate(status: &str) -> MollieMandate {
        serde_json::from_value(serde_json::json!({
            "id": "mdt_1",
            "status": status,
            "method": "creditcard",
            "customerId": "cst_1"
        }))
        .unwrap()
    }

    fn is_setup_pending(report: &Report<ConnectorError>) -> bool {
        report
            .frames()
            .any(|f| f.downcast_ref::<HostedSetupPending>().is_some())
    }

    #[tokio::test]
    async fn pending_mandate_is_reread_until_valid() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let reads = AtomicUsize::new(0);
        let statuses = ["pending", "pending", "valid"];
        let got = await_valid_mandate("mdt_1", &[std::time::Duration::ZERO; 4], || {
            let m = mandate(statuses[reads.fetch_add(1, SeqCst)]);
            async move { Ok(m) }
        })
        .await
        .unwrap();
        assert_eq!(got.status, MandateStatus::Valid);
        assert_eq!(reads.load(SeqCst), 3);
    }

    #[tokio::test]
    async fn mandate_still_pending_after_backoff_stays_retryable() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let reads = AtomicUsize::new(0);
        let err = await_valid_mandate("mdt_1", &[std::time::Duration::ZERO; 2], || {
            reads.fetch_add(1, SeqCst);
            async { Ok(mandate("pending")) }
        })
        .await
        .unwrap_err();
        assert!(is_setup_pending(&err), "must stay retryable: {err:?}");
        assert_eq!(reads.load(SeqCst), 3, "one read plus one per backoff step");
    }

    #[tokio::test]
    async fn invalid_mandate_fails_without_rereading() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let reads = AtomicUsize::new(0);
        let err = await_valid_mandate("mdt_1", &[std::time::Duration::ZERO; 3], || {
            reads.fetch_add(1, SeqCst);
            async { Ok(mandate("invalid")) }
        })
        .await
        .unwrap_err();
        assert!(!is_setup_pending(&err));
        assert!(matches!(
            err.current_context(),
            ConnectorError::MandateSetup(_)
        ));
        assert_eq!(reads.load(SeqCst), 1);
    }

    /// Seen in production: SEPA `first` on a profile without SEPA DD returns `creditcard`.
    #[test]
    fn swapped_in_method_is_detected() {
        let sepa: Vec<String> = SEPA_FIRST_PAYMENT_METHODS
            .iter()
            .map(|m| m.to_string())
            .collect();
        assert!(
            method_within_request(None, &sepa),
            "customer picks on Mollie"
        );
        assert!(method_within_request(Some("ideal"), &sepa));
        assert!(!method_within_request(Some("creditcard"), &sepa));
        let card = vec![CARD_METHOD.to_string()];
        assert!(method_within_request(Some("creditcard"), &card));
        assert!(!method_within_request(Some("ideal"), &card));
    }

    #[test]
    fn description_names_the_merchant_and_invoice() {
        use super::super::model::PaymentDescriptor;
        let invoice = PaymentDescriptor {
            merchant_name: "Macrosoft".into(),
            invoice_number: Some("INV-0052".into()),
        };
        assert_eq!(
            payment_description(Some(&invoice), &invoice_purpose(Some(&invoice))),
            "Macrosoft – Invoice INV-0052"
        );
        let setup = PaymentDescriptor {
            merchant_name: "Macrosoft".into(),
            invoice_number: None,
        };
        assert_eq!(
            payment_description(Some(&setup), "Card setup"),
            "Macrosoft – Card setup"
        );
        assert_eq!(invoice_purpose(Some(&setup)), "Payment");
        assert_eq!(
            payment_description(None, "Direct debit mandate"),
            "Direct debit mandate"
        );
        let blank = PaymentDescriptor {
            merchant_name: "  ".into(),
            invoice_number: None,
        };
        assert_eq!(
            payment_description(Some(&blank), "Subscription"),
            "Subscription"
        );

        let long = PaymentDescriptor {
            merchant_name: "M".repeat(300),
            invoice_number: None,
        };
        assert_eq!(
            payment_description(Some(&long), "Card setup")
                .chars()
                .count(),
            255
        );
    }

    fn hosted_payment(status: &str, cancelable: Option<bool>) -> MolliePayment {
        serde_json::from_value(serde_json::json!({
            "id": "tr_hosted",
            "status": status,
            "amount": {"currency": "EUR", "value": "10.00"},
            "sequenceType": "first",
            "isCancelable": cancelable
        }))
        .unwrap()
    }

    #[test]
    fn cancel_decision_never_orphans_a_capturable_payment() {
        for dead in ["failed", "canceled", "expired"] {
            assert_eq!(
                cancel_decision(&hosted_payment(dead, None)),
                CancelDecision::AlreadyDead
            );
        }
        assert_eq!(
            cancel_decision(&hosted_payment("open", Some(true))),
            CancelDecision::Cancel
        );
        assert_eq!(
            cancel_decision(&hosted_payment("open", Some(false))),
            CancelDecision::Adopt
        );
        assert_eq!(
            cancel_decision(&hosted_payment("open", None)),
            CancelDecision::Adopt
        );
        for captured in ["paid", "authorized"] {
            assert_eq!(
                cancel_decision(&hosted_payment(captured, Some(true))),
                CancelDecision::Adopt
            );
        }
    }

    #[test]
    fn failed_recurring_charge_rechecks_and_detaches_its_mandate() {
        let failed = payment(PaymentStatus::Failed, "10.00");
        assert!(needs_mandate_recheck(&failed));
        assert!(!needs_mandate_recheck(&payment(
            PaymentStatus::Paid,
            "10.00"
        )));
        let mut first = payment(PaymentStatus::Failed, "10.00");
        first.sequence_type = Some(SequenceType::First);
        assert!(
            !needs_mandate_recheck(&first),
            "setup failures carry no live mandate"
        );

        let event = mandate_detached_event(&failed, "mdt_1");
        assert_eq!(event.owner_tenant_id.as_deref(), Some("tenantB62"));
        match event.kind {
            NormalizedEventKind::PaymentMethodDetached(e) => {
                assert_eq!(e.external_payment_method_id, "mdt_1")
            }
            other => panic!("expected PaymentMethodDetached, got {other:?}"),
        }
    }

    #[test]
    fn refused_charge_is_a_terminal_decline() {
        assert!(is_rejected_charge(422));
        assert!(is_rejected_charge(404));
        assert!(is_rejected_charge(400));
        for not_a_decline in [401, 403, 408, 409, 429, 500, 503] {
            assert!(!is_rejected_charge(not_a_decline), "{not_a_decline}");
        }

        let err = mollie_client::error::RequestError {
            status: 422,
            title: "Unprocessable Entity".into(),
            detail: "The mandate is invalid".into(),
            field: Some("mandateId".into()),
        };
        let revoked = rejected_charge(&err, true);
        assert!(!revoked.retryable);
        assert!(matches!(revoked.decline_kind, DeclineKind::MandateInactive));
        assert_eq!(revoked.code.as_deref(), Some("mandate_invalid"));

        let other = rejected_charge(&err, false);
        assert!(!other.retryable);
        assert!(matches!(other.decline_kind, DeclineKind::Other));
        assert_eq!(other.code.as_deref(), Some("mollie_422"));
    }

    fn payment(status: PaymentStatus, value: &str) -> MolliePayment {
        serde_json::from_value(serde_json::json!({
            "id": "tr_test1",
            "status": format!("{status:?}").to_lowercase(),
            "amount": {"currency": "EUR", "value": value},
            "sequenceType": "recurring",
            "customerId": "cst_1",
            "mandateId": "mdt_1",
            "metadata": {"meteroid.tenant_id": "tenantB62", "meteroid.transaction_id": "txB62"},
            "paidAt": "2026-09-11T10:00:00+00:00"
        }))
        .unwrap()
    }

    fn with_failure(mut payment: MolliePayment, reason: &str) -> MolliePayment {
        payment.details = Some(PaymentDetails {
            failure_reason: Some(reason.into()),
            failure_message: Some("declined".into()),
            ..Default::default()
        });
        payment
    }

    #[test]
    fn capabilities_match_provider_reality() {
        let caps = MollieConnector::new().capabilities().clone();
        assert!(caps.supports_cards);
        assert!(caps.supports_mandates);
        assert!(!caps.supports_refunds);
        assert!(caps.supports_3ds);
        assert!(caps.supports_disputes);
        assert!(!caps.supports_self_webhook_registration);
        assert!(caps.asynchronous_settlement);
        assert_eq!(caps.mandate_setup_mode, MandateSetupMode::HostedRedirect);
        assert_eq!(
            caps.hosted_setup_completion,
            HostedSetupCompletion::WebhookBacked
        );
    }

    #[test]
    fn webhook_url_carries_the_token() {
        let connector = test_connector();
        let endpoint = "https://api.example.invalid/webhooks/v1/7ZKs3fwYtrG/mollie-test";
        assert_eq!(
            webhook_url(&connector, Some(endpoint)).as_deref(),
            Some(&*format!("{endpoint}?token={TOKEN}"))
        );
        assert_eq!(
            webhook_url(&connector, None),
            None,
            "no endpoint, no payment"
        );
        let mut not_mollie = test_connector();
        not_mollie.sensitive = None;
        assert_eq!(webhook_url(&not_mollie, Some(endpoint)), None);
    }

    /// A notification has no event id; it must be enqueued as one non-deduplicated unit.
    #[test]
    fn form_ping_ingests_as_one_undeduped_unit() {
        let connector = test_connector();
        let body = b"id=tr_7UhSN1zuXS";
        let units = MollieConnector::new()
            .split_delivery(&connector, body)
            .unwrap();
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].event_id, None);
        assert_eq!(units[0].body, body.to_vec());
    }

    #[test]
    fn query_param_helpers() {
        assert_eq!(
            with_query_param("https://x.invalid/r?connection=c", "intent", "tr_1"),
            "https://x.invalid/r?connection=c&intent=tr_1"
        );
        assert_eq!(
            with_query_param("https://x.invalid/r", "error", "flow_abandoned"),
            "https://x.invalid/r?error=flow_abandoned"
        );
        assert_eq!(
            query_param("a=1&token=abc&b=2", "token").as_deref(),
            Some("abc")
        );
        assert_eq!(query_param("a=1", "token"), None);
    }

    #[test]
    fn classic_ping_is_authenticated_by_the_url_token() {
        let connector = test_connector();
        let mollie = MollieConnector::new();
        let secret = SecretString::from(TOKEN.to_string());
        let body = b"id=tr_abc";

        let mut headers = HeaderMap::new();
        headers.insert(
            REQUEST_QUERY_HEADER,
            format!("token={TOKEN}").parse().unwrap(),
        );
        assert!(
            mollie
                .verify_signature(&connector, body, &headers, &secret)
                .is_ok()
        );

        let mut wrong = HeaderMap::new();
        wrong.insert(REQUEST_QUERY_HEADER, "token=nope".parse().unwrap());
        assert!(matches!(
            mollie
                .verify_signature(&connector, body, &wrong, &secret)
                .unwrap_err()
                .current_context(),
            ConnectorError::SignatureVerification
        ));

        assert!(matches!(
            mollie
                .verify_signature(&connector, body, &HeaderMap::new(), &secret)
                .unwrap_err()
                .current_context(),
            ConnectorError::SignatureMissing
        ));
    }

    #[test]
    fn parse_events_reduces_pings_to_resource_changed() {
        let connector = test_connector();
        let mollie = MollieConnector::new();

        let ping = mollie
            .parse_events(&connector, b"id=tr_ping", &HeaderMap::new())
            .unwrap();
        assert_eq!(ping.len(), 1);
        assert!(matches!(
            &ping[0].kind,
            NormalizedEventKind::ResourceChanged { resource_ref } if resource_ref == "tr_ping"
        ));

        assert!(matches!(
            mollie
                .parse_events(&connector, b"garbage", &HeaderMap::new())
                .unwrap_err()
                .current_context(),
            ConnectorError::PayloadDecode(_)
        ));
    }

    #[test]
    fn charge_outcome_table() {
        match payment_to_outcome(&payment(PaymentStatus::Paid, "42.00")).unwrap() {
            ChargeOutcome::Succeeded(r) => {
                assert_eq!(r.external_id, "tr_test1");
                assert_eq!(r.amount_received_minor, 4_200);
            }
            other => panic!("expected Succeeded, got {other:?}"),
        }
        for status in [
            PaymentStatus::Open,
            PaymentStatus::Pending,
            PaymentStatus::Authorized,
            PaymentStatus::Unknown,
        ] {
            assert!(
                matches!(
                    payment_to_outcome(&payment(status, "1.00")).unwrap(),
                    ChargeOutcome::Pending(_)
                ),
                "{status:?} must be Pending"
            );
        }
        assert!(matches!(
            payment_to_outcome(&payment(PaymentStatus::Canceled, "1.00")).unwrap(),
            ChargeOutcome::Cancelled(_)
        ));
        match payment_to_outcome(&with_failure(
            payment(PaymentStatus::Failed, "1.00"),
            "insufficient_funds",
        ))
        .unwrap()
        {
            ChargeOutcome::Failed(f) => {
                assert!(!f.retryable);
                assert_eq!(f.code.as_deref(), Some("insufficient_funds"));
                assert_eq!(f.decline_kind, DeclineKind::InsufficientFunds);
                assert_eq!(f.message, "declined");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(matches!(
            payment_to_outcome(&payment(PaymentStatus::Expired, "1.00")).unwrap(),
            ChargeOutcome::Failed(_)
        ));
    }

    #[test]
    fn decline_kind_map() {
        assert_eq!(
            decline_kind_for(Some("insufficient_funds")),
            DeclineKind::InsufficientFunds
        );
        assert_eq!(
            decline_kind_for(Some("card_expired")),
            DeclineKind::CardExpired
        );
        assert_eq!(
            decline_kind_for(Some("inactive_card")),
            DeclineKind::CardExpired
        );
        assert_eq!(
            decline_kind_for(Some("refused_by_issuer")),
            DeclineKind::DoNotHonor
        );
        assert_eq!(decline_kind_for(Some("possible_fraud")), DeclineKind::Fraud);
        assert_eq!(
            decline_kind_for(Some("authentication_failed")),
            DeclineKind::AuthenticationRequired
        );
        assert_eq!(decline_kind_for(Some("unknown_reason")), DeclineKind::Other);
        assert_eq!(decline_kind_for(None), DeclineKind::Other);
    }

    #[test]
    fn remote_status_table() {
        match remote_status_from_payment(&payment(PaymentStatus::Paid, "42.00")).unwrap() {
            RemoteTransactionStatus::Succeeded {
                amount_received_minor,
                currency,
                processed_at,
            } => {
                assert_eq!(amount_received_minor, 4_200);
                assert_eq!(currency, "EUR");
                assert_eq!(processed_at.to_string(), "2026-09-11 10:00:00");
            }
            other => panic!("expected Succeeded, got {other:?}"),
        }
        assert!(matches!(
            remote_status_from_payment(&payment(PaymentStatus::Pending, "1.00")).unwrap(),
            RemoteTransactionStatus::Pending
        ));
        assert!(matches!(
            remote_status_from_payment(&payment(PaymentStatus::Canceled, "1.00")).unwrap(),
            RemoteTransactionStatus::Cancelled
        ));
        match remote_status_from_payment(&with_failure(
            payment(PaymentStatus::Failed, "1.00"),
            "card_expired",
        ))
        .unwrap()
        {
            RemoteTransactionStatus::Failed {
                code, decline_kind, ..
            } => {
                assert_eq!(code.as_deref(), Some("card_expired"));
                assert_eq!(decline_kind, DeclineKind::CardExpired);
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn only_an_open_setup_reads_not_completed() {
        let has = |r: &Report<ConnectorError>, not_completed: bool| {
            assert!(
                r.frames()
                    .any(|f| f.downcast_ref::<HostedSetupPending>().is_some())
            );
            assert_eq!(
                r.frames()
                    .any(|f| f.downcast_ref::<HostedSetupNotCompleted>().is_some()),
                not_completed
            );
        };
        has(&pending_setup_error("tr_1", &PaymentStatus::Open), true);
        for status in [
            PaymentStatus::Pending,
            PaymentStatus::Authorized,
            PaymentStatus::Paid,
            PaymentStatus::Unknown,
        ] {
            has(&pending_setup_error("tr_1", &status), false);
        }
    }

    #[test]
    fn setup_state_table() {
        let mut first = payment(PaymentStatus::Paid, "0.00");
        first.sequence_type = Some(SequenceType::First);
        assert_eq!(setup_state(&first), SetupState::Completed);
        // An uncaptured authorization can still expire: not complete.
        first.status = PaymentStatus::Authorized;
        assert_eq!(setup_state(&first), SetupState::Pending);
        first.status = PaymentStatus::Open;
        assert_eq!(setup_state(&first), SetupState::Pending);
        first.status = PaymentStatus::Unknown;
        assert_eq!(setup_state(&first), SetupState::Pending);
        first.status = PaymentStatus::Paid;
        first.mandate_id = None;
        assert_eq!(setup_state(&first), SetupState::Pending);
        for dead in [
            PaymentStatus::Failed,
            PaymentStatus::Canceled,
            PaymentStatus::Expired,
        ] {
            first.status = dead;
            assert_eq!(setup_state(&first), SetupState::Failed);
        }
    }

    #[test]
    fn snapshot_recovers_mandate_and_metadata() {
        let mandate = MollieMandate {
            id: "mdt_1".into(),
            mode: None,
            status: MandateStatus::Valid,
            method: Some("creditcard".into()),
            customer_id: Some("cst_1".into()),
            details: Some(MandateDetails {
                card_number: Some("4242".into()),
                card_label: Some("Visa".into()),
                card_expiry_date: Some("2030-12-31".into()),
                card_fingerprint: Some("fHB3CCKx9REkz8fPplT8N4nq".into()),
                ..Default::default()
            }),
            mandate_reference: None,
            signature_date: None,
            created_at: None,
        };
        let metadata = HashMap::from([
            ("meteroid.connection_id".to_string(), "conn_x".to_string()),
            ("meteroid.customer_id".to_string(), "cust_x".to_string()),
            (
                "meteroid.checkout_session_id".to_string(),
                "sess_x".to_string(),
            ),
            ("meteroid.transaction_id".to_string(), "tx_x".to_string()),
        ]);
        let snapshot = snapshot_from_mandate(mandate, &metadata, Some("tr_first".into()));
        assert_eq!(snapshot.external_payment_method_id, "mdt_1");
        assert_eq!(snapshot.payment_method_type, PaymentMethodTypeEnum::Card);
        assert_eq!(snapshot.card_brand.as_deref(), Some("Visa"));
        assert_eq!(snapshot.card_last4.as_deref(), Some("4242"));
        assert_eq!(snapshot.card_exp_month, Some(12));
        assert_eq!(snapshot.card_exp_year, Some(2030));
        assert_eq!(
            snapshot.fingerprint.as_deref(),
            Some("fHB3CCKx9REkz8fPplT8N4nq")
        );
        assert_eq!(snapshot.meteroid_connection_id.as_deref(), Some("conn_x"));
        assert_eq!(snapshot.meteroid_customer_id.as_deref(), Some("cust_x"));
        assert_eq!(
            snapshot.meteroid_checkout_session_id.as_deref(),
            Some("sess_x")
        );
        assert_eq!(snapshot.meteroid_transaction_id.as_deref(), Some("tx_x"));
        assert_eq!(
            snapshot.payment_request_payment.as_deref(),
            Some("tr_first")
        );
        assert!(snapshot.meteroid_invoice_id.is_none());

        assert_eq!(parse_expiry("12/28"), Some((2028, 12)));
        assert_eq!(parse_expiry("2030-01-15"), Some((2030, 1)));
        assert_eq!(parse_expiry("garbage"), None);
    }

    #[test]
    fn sepa_mandate_fingerprint_is_the_normalized_iban_hash() {
        let sepa = |iban: &str| MollieMandate {
            id: "mdt_sepa".into(),
            mode: None,
            status: MandateStatus::Valid,
            method: Some("directdebit".into()),
            customer_id: None,
            details: Some(MandateDetails {
                consumer_account: Some(iban.into()),
                ..Default::default()
            }),
            mandate_reference: None,
            signature_date: None,
            created_at: None,
        };
        let a = snapshot_from_mandate(sepa("NL91 ABNA 0417 1643 00"), &HashMap::new(), None);
        let b = snapshot_from_mandate(sepa("nl91abna0417164300"), &HashMap::new(), None);
        let c = snapshot_from_mandate(sepa("NL91ABNA0417164301"), &HashMap::new(), None);
        assert_eq!(
            a.payment_method_type,
            PaymentMethodTypeEnum::DirectDebitSepa
        );
        assert_eq!(a.account_number_hint.as_deref(), Some("4300"));
        let fp = a.fingerprint.expect("sepa mandates are fingerprinted");
        assert!(fp.starts_with("iban:"));
        assert!(!fp.contains("4300"));
        assert_eq!(b.fingerprint.as_deref(), Some(fp.as_str()));
        assert_ne!(c.fingerprint.as_deref(), Some(fp.as_str()));
    }

    #[test]
    fn setup_metadata_carries_the_right_ids() {
        use super::super::model::{HostedCheckoutContext, IdempotencyKey};
        use common_domain::ids::{CustomerConnectionId, CustomerId};

        let connector = test_connector();
        let connection = CustomerConnection {
            id: CustomerConnectionId::new(),
            customer_id: CustomerId::new(),
            connector_id: connector.id,
            external_customer_id: "cst_ext".into(),
            supported_payment_types: Some(vec![PaymentMethodTypeEnum::Card]),
        };
        let request = |invoice_id: Option<String>, checkout: Option<HostedCheckoutContext>| {
            MandateSetupRequest {
                descriptor: None,
                webhook_url: None,
                payment_methods: &[PaymentMethodTypeEnum::Card],
                idempotency_key: IdempotencyKey::new("k"),
                return_url: Some("https://api.example.invalid/return".into()),
                invoice_id,
                checkout,
                invoice_payment: None,
                currency: Some("EUR".into()),
            }
        };

        let checkout = request(
            None,
            Some(HostedCheckoutContext {
                tenant_id: connector.tenant_id.as_base62(),
                checkout_session_id: "sessB62".into(),
                transaction_id: "txB62".into(),
                amount_minor: 4_200,
                currency: "EUR".into(),
            }),
        );
        let metadata = setup_payment_metadata(&connector, &connection, &checkout);
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
            metadata.get("meteroid.tenant_id"),
            Some(&connector.tenant_id.as_base62())
        );
        assert!(!metadata.contains_key("meteroid.invoice_id"));

        let invoice = request(Some("invB62".into()), None);
        let metadata = setup_payment_metadata(&connector, &connection, &invoice);
        assert_eq!(
            metadata.get("meteroid.invoice_id").map(String::as_str),
            Some("invB62")
        );
        assert!(!metadata.contains_key("meteroid.transaction_id"));
    }

    #[test]
    fn events_for_first_payment() {
        let mut save = payment(PaymentStatus::Paid, "0.00");
        save.sequence_type = Some(SequenceType::First);
        save.metadata.remove("meteroid.transaction_id");
        let events = events_for_payment(&save, &[]).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0].kind,
            NormalizedEventKind::MandateSetupCompleted { provider_intent_id } if provider_intent_id == "tr_test1"
        ));
        assert_eq!(events[0].owner_tenant_id.as_deref(), Some("tenantB62"));

        let mut checkout = payment(PaymentStatus::Paid, "42.00");
        checkout.sequence_type = Some(SequenceType::First);
        let events = events_for_payment(&checkout, &[]).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0].kind,
            NormalizedEventKind::MandateSetupCompleted { .. }
        ));
        match &events[1].kind {
            NormalizedEventKind::PaymentSucceeded(e) => {
                assert_eq!(e.amount_received_minor, 4_200);
                assert_eq!(e.meteroid_transaction_id.as_deref(), Some("txB62"));
            }
            other => panic!("expected PaymentSucceeded, got {other:?}"),
        }

        let mut open_save = payment(PaymentStatus::Open, "0.00");
        open_save.sequence_type = Some(SequenceType::First);
        open_save.metadata.remove("meteroid.transaction_id");
        assert!(matches!(
            events_for_payment(&open_save, &[]).unwrap()[0].kind,
            NormalizedEventKind::Acknowledged { .. }
        ));

        let mut declined_checkout =
            with_failure(payment(PaymentStatus::Failed, "42.00"), "card_declined");
        declined_checkout.sequence_type = Some(SequenceType::First);
        let events = events_for_payment(&declined_checkout, &[]).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].kind,
            NormalizedEventKind::PaymentFailed(_)
        ));
    }

    #[test]
    fn events_for_recurring_payment_and_reversals() {
        let paid = payment(PaymentStatus::Paid, "10.00");
        let events = events_for_payment(&paid, &[]).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].provider_event_type, "payment.paid");
        assert!(matches!(
            events[0].kind,
            NormalizedEventKind::PaymentSucceeded(_)
        ));

        assert!(matches!(
            events_for_payment(&payment(PaymentStatus::Pending, "10.00"), &[]).unwrap()[0].kind,
            NormalizedEventKind::PaymentPending(_)
        ));
        match &events_for_payment(&payment(PaymentStatus::Canceled, "10.00"), &[]).unwrap()[0].kind
        {
            NormalizedEventKind::PaymentFailed(e) => {
                assert_eq!(e.code.as_deref(), Some("canceled"))
            }
            other => panic!("expected PaymentFailed, got {other:?}"),
        }

        let mut refunded = payment(PaymentStatus::Paid, "10.00");
        refunded.amount_refunded = Some(Amount::from_minor(400, "EUR", 2));
        refunded.amount_charged_back = Some(Amount::from_minor(1000, "EUR", 2));
        let chargebacks: Vec<MollieChargeback> = serde_json::from_value(serde_json::json!([
            {"id": "chb_1", "amount": {"currency": "EUR", "value": "10.00"}, "paymentId": "tr_test1",
             "reason": {"code": "AC01", "description": "Incorrect account number"},
             "createdAt": "2026-09-01T08:00:00+00:00", "reversedAt": null},
            {"id": "chb_0", "amount": {"currency": "EUR", "value": "10.00"}, "paymentId": "tr_test1",
             "createdAt": "2026-08-20T08:00:00+00:00", "reversedAt": "2026-08-25T08:00:00+00:00"}
        ]))
        .unwrap();
        let events = events_for_payment(&refunded, &chargebacks).unwrap();
        assert_eq!(events.len(), 4);
        // Sorted by date regardless of Mollie's listing order.
        assert!(
            events[1..]
                .windows(2)
                .all(|w| w[0].occurred_at <= w[1].occurred_at),
            "reversals must be ordered by occurred_at"
        );
        assert!(matches!(
            events[1].kind,
            NormalizedEventKind::DisputeFundsReinstated(_)
        ));
        assert_eq!(
            events[1].occurred_at.to_rfc3339(),
            "2026-08-25T08:00:00+00:00"
        );
        match &events[2].kind {
            NormalizedEventKind::DisputeFundsWithdrawn(d) => {
                assert_eq!(d.external_dispute_id, "chb_1");
                assert_eq!(d.amount_minor, 1000);
                assert_eq!(d.reason.as_deref(), Some("AC01"));
            }
            other => panic!("expected DisputeFundsWithdrawn, got {other:?}"),
        }
        assert_eq!(
            events[2].occurred_at.to_rfc3339(),
            "2026-09-01T08:00:00+00:00"
        );
        match &events[3].kind {
            NormalizedEventKind::PaymentRefunded(e) => assert_eq!(e.amount_refunded_minor, 400),
            other => panic!("expected PaymentRefunded, got {other:?}"),
        }
    }

    #[test]
    fn unparseable_amount_is_an_error_not_zero() {
        let mut checkout = payment(PaymentStatus::Paid, "42.005");
        checkout.sequence_type = Some(SequenceType::First);
        assert!(events_for_payment(&checkout, &[]).is_err());
        assert!(payment_to_outcome(&checkout).is_err());
        assert!(remote_status_from_payment(&checkout).is_err());
        let mut unknown_currency = payment(PaymentStatus::Paid, "10.00");
        unknown_currency.amount.currency = "XXX".into();
        assert!(events_for_payment(&unknown_currency, &[]).is_err());
    }

    #[test]
    fn zero_decimal_currency_round_trip() {
        let amount = amount_from_minor(1500, "JPY").unwrap();
        assert_eq!(amount.value.to_string(), "1500");
        let mut paid = payment(PaymentStatus::Paid, "1500");
        paid.amount = amount;
        match &events_for_payment(&paid, &[]).unwrap()[0].kind {
            NormalizedEventKind::PaymentSucceeded(e) => {
                assert_eq!(e.amount_received_minor, 1500);
                assert_eq!(e.currency, "JPY");
            }
            other => panic!("expected PaymentSucceeded, got {other:?}"),
        }
        assert_eq!(
            amount_from_minor(1999, "eur").unwrap().value.to_string(),
            "19.99"
        );
        assert!(amount_from_minor(1, "XXX").is_err());
    }

    #[test]
    fn rail_selection_and_charge_rules() {
        assert_eq!(
            setup_rail(&[PaymentMethodTypeEnum::Card]),
            Ok(SetupRail::Card)
        );
        assert_eq!(
            setup_rail(&[PaymentMethodTypeEnum::DirectDebitSepa]),
            Ok(SetupRail::Sepa)
        );
        assert_eq!(
            setup_rail(&[
                PaymentMethodTypeEnum::DirectDebitSepa,
                PaymentMethodTypeEnum::Card
            ]),
            Ok(SetupRail::Card)
        );
        assert!(setup_rail(&[PaymentMethodTypeEnum::DirectDebitBacs]).is_err());
        assert!(setup_rail(&[PaymentMethodTypeEnum::DirectDebitAch]).is_err());
        assert!(setup_rail(&[]).is_err());

        assert!(validate_charge_rail(&PaymentMethodTypeEnum::Card, "USD").is_ok());
        assert!(validate_charge_rail(&PaymentMethodTypeEnum::DirectDebitSepa, "eur").is_ok());
        assert!(validate_charge_rail(&PaymentMethodTypeEnum::DirectDebitSepa, "USD").is_err());
        assert!(validate_charge_rail(&PaymentMethodTypeEnum::DirectDebitAch, "USD").is_err());
        assert!(validate_charge_rail(&PaymentMethodTypeEnum::DirectDebitBacs, "gbp").is_err());
    }

    #[test]
    fn sepa_verification_payment_is_setup_only() {
        let mut verification: MolliePayment = serde_json::from_value(serde_json::json!({
            "id": "tr_sepa1",
            "status": "paid",
            "amount": {"currency": "EUR", "value": "0.01"},
            "sequenceType": "first",
            "method": "ideal",
            "customerId": "cst_1",
            "mandateId": "mdt_dd",
            "metadata": {"meteroid.tenant_id": "tenantB62", "meteroid.connection_id": "connB62"},
            "paidAt": "2026-09-11T10:00:00+00:00"
        }))
        .unwrap();
        let events = events_for_payment(&verification, &[]).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].kind,
            NormalizedEventKind::MandateSetupCompleted { .. }
        ));

        verification.status = PaymentStatus::Expired;
        let events = events_for_payment(&verification, &[]).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].kind,
            NormalizedEventKind::Acknowledged { .. }
        ));
    }

    #[tokio::test]
    async fn sepa_setup_and_charge_reject_non_eur() {
        use super::super::model::{HostedCheckoutContext, IdempotencyKey};
        use common_domain::ids::{CustomerConnectionId, CustomerId, PaymentTransactionId};

        let connector = test_connector();
        let connection = CustomerConnection {
            id: CustomerConnectionId::new(),
            customer_id: CustomerId::new(),
            connector_id: connector.id,
            external_customer_id: "cst_ext".into(),
            supported_payment_types: Some(vec![PaymentMethodTypeEnum::DirectDebitSepa]),
        };
        let mollie = MollieConnector::new();
        let usd_checkout = mollie
            .initiate_mandate_setup(
                &connector,
                &connection,
                MandateSetupRequest {
                    descriptor: None,
                    webhook_url: None,
                    payment_methods: &[PaymentMethodTypeEnum::DirectDebitSepa],
                    idempotency_key: IdempotencyKey::new("k"),
                    return_url: Some("https://api.example.invalid/return".into()),
                    invoice_id: None,
                    checkout: Some(HostedCheckoutContext {
                        tenant_id: connector.tenant_id.as_base62(),
                        checkout_session_id: "sessB62".into(),
                        transaction_id: "txB62".into(),
                        amount_minor: 4_200,
                        currency: "USD".into(),
                    }),
                    invoice_payment: None,
                    currency: Some("USD".into()),
                },
            )
            .await;
        assert!(matches!(
            usd_checkout.as_ref().err().map(|r| r.current_context()),
            Some(ConnectorError::MandateSetup(_))
        ));

        let usd_charge = mollie
            .charge_off_session(
                &connector,
                ChargeRequest {
                    descriptor: None,
                    webhook_url: None,
                    transaction_id: PaymentTransactionId::new(),
                    customer_external_id: "cst_ext",
                    payment_method_external_id: "mdt_dd",
                    payment_method_type: PaymentMethodTypeEnum::DirectDebitSepa,
                    amount_minor: 100,
                    currency: "USD",
                    idempotency_key: IdempotencyKey::new("k"),
                    on_session: false,
                },
            )
            .await;
        assert!(matches!(usd_charge, Ok(ChargeOutcome::Failed(_))));
    }

    #[test]
    fn hosted_capture_and_metadata_for_an_in_flow_invoice() {
        use super::super::model::{HostedInvoicePaymentContext, IdempotencyKey};
        use common_domain::ids::{CustomerConnectionId, CustomerId};

        let connector = test_connector();
        let connection = CustomerConnection {
            id: CustomerConnectionId::new(),
            customer_id: CustomerId::new(),
            connector_id: connector.id,
            external_customer_id: "cst_ext".into(),
            supported_payment_types: None,
        };
        let request = MandateSetupRequest {
            descriptor: None,
            webhook_url: None,
            payment_methods: &[PaymentMethodTypeEnum::DirectDebitSepa],
            idempotency_key: IdempotencyKey::new("k"),
            return_url: Some("https://api.example.invalid/return".into()),
            invoice_id: Some("invB62".into()),
            checkout: None,
            invoice_payment: Some(HostedInvoicePaymentContext {
                invoice_id: "invB62".into(),
                transaction_id: "txB62".into(),
                amount_minor: 12_000,
                currency: "EUR".into(),
            }),
            currency: Some("EUR".into()),
        };
        assert_eq!(hosted_capture(&request), Some((12_000, "EUR")));
        let metadata = setup_payment_metadata(&connector, &connection, &request);
        assert_eq!(
            metadata.get("meteroid.transaction_id").map(String::as_str),
            Some("txB62")
        );
        assert_eq!(
            metadata.get("meteroid.invoice_id").map(String::as_str),
            Some("invB62")
        );
        assert!(!metadata.contains_key("meteroid.checkout_session_id"));
    }

    #[tokio::test]
    async fn payments_require_a_webhook_url() {
        use super::super::model::IdempotencyKey;
        use common_domain::ids::{CustomerConnectionId, CustomerId, PaymentTransactionId};

        let mut connector = test_connector();
        connector.data = Some(ProviderData::Mollie(MolliePublicData::default()));
        let connection = CustomerConnection {
            id: CustomerConnectionId::new(),
            customer_id: CustomerId::new(),
            connector_id: connector.id,
            external_customer_id: "cst_ext".into(),
            supported_payment_types: Some(vec![PaymentMethodTypeEnum::Card]),
        };
        let mollie = MollieConnector::new();
        let setup = mollie
            .initiate_mandate_setup(
                &connector,
                &connection,
                MandateSetupRequest {
                    descriptor: None,
                    webhook_url: None,
                    payment_methods: &[PaymentMethodTypeEnum::Card],
                    idempotency_key: IdempotencyKey::new("k"),
                    return_url: Some("https://api.example.invalid/return".into()),
                    invoice_id: None,
                    checkout: None,
                    invoice_payment: None,
                    currency: Some("EUR".into()),
                },
            )
            .await;
        assert!(matches!(
            setup.as_ref().err().map(|r| r.current_context()),
            Some(ConnectorError::Configuration(_))
        ));
        let charge = mollie
            .charge_off_session(
                &connector,
                ChargeRequest {
                    descriptor: None,
                    webhook_url: None,
                    transaction_id: PaymentTransactionId::new(),
                    customer_external_id: "cst_ext",
                    payment_method_external_id: "mdt_1",
                    payment_method_type: PaymentMethodTypeEnum::Card,
                    amount_minor: 100,
                    currency: "EUR",
                    idempotency_key: IdempotencyKey::new("k"),
                    on_session: false,
                },
            )
            .await;
        assert!(matches!(
            charge.as_ref().err().map(|r| r.current_context()),
            Some(ConnectorError::Configuration(_))
        ));
    }

    #[tokio::test]
    async fn charges_mollie_cannot_accept_fail_without_calling_mollie() {
        let connector = test_connector();
        let mollie = MollieConnector::new();
        for (method, amount_minor, currency) in [
            (PaymentMethodTypeEnum::DirectDebitSepa, 100, "USD"),
            (PaymentMethodTypeEnum::DirectDebitBacs, 100, "GBP"),
            (PaymentMethodTypeEnum::Card, 0, "EUR"),
        ] {
            let outcome = mollie
                .charge_off_session(
                    &connector,
                    ChargeRequest {
                        descriptor: None,
                        webhook_url: None,
                        transaction_id: common_domain::ids::PaymentTransactionId::new(),
                        customer_external_id: "cst_ext",
                        payment_method_external_id: "mdt_1",
                        payment_method_type: method.clone(),
                        amount_minor,
                        currency,
                        idempotency_key: crate::adapters::payment::model::IdempotencyKey::new("k"),
                        on_session: false,
                    },
                )
                .await
                .expect("a failed outcome, not an error");
            assert!(
                matches!(outcome, ChargeOutcome::Failed(_)),
                "{method:?} {currency}"
            );
        }
    }

    #[test]
    fn auth_errors_are_configuration_not_terminal_setup_failure() {
        use mollie_client::error::RequestError;

        let err_with_status = |status: u16| {
            MollieError::Mollie(RequestError {
                status,
                title: String::new(),
                detail: "x".into(),
                field: None,
            })
        };
        for status in [401u16, 403] {
            let report = map_mollie_error(MollieOp::Mandate, err_with_status(status));
            assert!(matches!(
                report.current_context(),
                ConnectorError::Configuration(_)
            ));
        }
        for status in [408u16, 409, 429, 500, 503] {
            let report = map_mollie_error(MollieOp::Charge, err_with_status(status));
            assert!(
                matches!(report.current_context(), ConnectorError::Transport(_)),
                "{status} must map to retryable Transport"
            );
        }
        for status in [404u16, 422] {
            let report = map_mollie_error(MollieOp::Mandate, err_with_status(status));
            assert!(matches!(
                report.current_context(),
                ConnectorError::MandateSetup(_)
            ));
        }
        assert!(matches!(
            map_mollie_error(MollieOp::Customer, err_with_status(422)).current_context(),
            ConnectorError::CustomerOp(_)
        ));
    }

    #[tokio::test]
    async fn unsupported_ops_error_instead_of_panic() {
        let connector = test_connector();
        let mollie = MollieConnector::new();
        let refund = mollie
            .refund(
                &connector,
                RefundRequest {
                    external_transaction_id: "tr_x",
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
        let registered = mollie
            .register_webhook(&connector, "https://x.invalid", &[])
            .await;
        assert!(matches!(
            registered.as_ref().err().map(|r| r.current_context()),
            Some(ConnectorError::Unsupported { .. })
        ));
    }

    #[tokio::test]
    async fn initiate_setup_rejects_unsupported_rails() {
        use super::super::model::IdempotencyKey;
        use common_domain::ids::{CustomerConnectionId, CustomerId};

        let connector = test_connector();
        let connection = CustomerConnection {
            id: CustomerConnectionId::new(),
            customer_id: CustomerId::new(),
            connector_id: connector.id,
            external_customer_id: "cst_ext".into(),
            supported_payment_types: Some(vec![PaymentMethodTypeEnum::Card]),
        };
        let mollie = MollieConnector::new();
        let dd = mollie
            .initiate_mandate_setup(
                &connector,
                &connection,
                MandateSetupRequest {
                    descriptor: None,
                    webhook_url: None,
                    payment_methods: &[PaymentMethodTypeEnum::DirectDebitAch],
                    idempotency_key: IdempotencyKey::new("k"),
                    return_url: Some("https://api.example.invalid/return".into()),
                    invoice_id: None,
                    checkout: None,
                    invoice_payment: None,
                    currency: Some("EUR".into()),
                },
            )
            .await;
        assert!(matches!(
            dd.as_ref().err().map(|r| r.current_context()),
            Some(ConnectorError::MandateSetup(_))
        ));
    }
}
