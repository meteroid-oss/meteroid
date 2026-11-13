//! Mock connector: the in-test payment double and, when the instance enables it, the Test gateway.
//! The adapter's [`MockPublicData`] wins (how tests force an outcome); otherwise the
//! scenario encoded in the payment method id decides (see [`MockPaymentMethodRef`]).

use super::account::ValidatedCredentials;
use super::connector::{
    ConnectorIdentity, ConnectorRef, CredentialOps, CustomerOps, InboundWebhooks, MandateOps,
    MandateSetup, PaymentCapabilities, PaymentOps, ReconcileOps, RefundOps, RefundSupport,
    WalletOps, WebhookOps,
};
use super::error::ConnectorError;
use super::events::{
    DisputeEvent, NormalizedEventKind, NormalizedEventSubscription, NormalizedWebhookEvent,
    ObservedRefundOnWire, PaymentFailedEvent, PaymentMethodAttachedEvent,
    PaymentMethodUpdatedEvent, PaymentPendingEvent, PaymentRefundedEvent, PaymentReinstatedEvent,
    PaymentRequiresActionEvent, PaymentReversedEvent, PaymentSucceededEvent,
};
use super::model::{
    ChargeAcknowledged, ChargeCancelled, ChargeFailure, ChargeOutcome, ChargeReceipt,
    ChargeRequest, CreateCustomerRequest, DeclineKind, ExternalCustomerRef,
    MandateSetupInstruction, MandateSetupRequest, PaymentMethodSnapshot,
    PrepareWalletChargeRequest, RefundAcknowledged, RefundFailure, RefundOutcome, RefundReceipt,
    RefundRequest, RefundSnapshot, RegisteredWebhook, RemoteRefundStatus, RemoteTransactionStatus,
    RequiresActionInstruction,
};
use crate::domain::ObservedRefundStatus;
use crate::domain::connectors::{
    MethodState, MockChargeBehavior, MockPublicData, MockReconcileStatus, MockRefundBehavior,
    PaymentConnectorPublicData, PaymentMethodAvailability, ProviderAccount,
};
use crate::domain::enums::{PaymentProvider, ReversalKind};
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::{Customer, CustomerConnection, PaymentMethodTypeEnum};
use async_trait::async_trait;
use common_domain::ids::BaseId;
use error_stack::Report;
use http::HeaderMap;
use secrecy::SecretString;
use std::num::NonZeroU32;
use std::time::Duration;
use uuid::Uuid;

pub(super) const MOCK_CAPABILITIES: PaymentCapabilities = PaymentCapabilities {
    supported_payment_methods: &[
        PaymentMethodTypeEnum::Card,
        PaymentMethodTypeEnum::DirectDebitSepa,
        PaymentMethodTypeEnum::DirectDebitAch,
        PaymentMethodTypeEnum::DirectDebitBacs,
    ],
    supports_mandates: true,
    refunds: RefundSupport::Partial,
    // The mock keys its simulated refunds the same way a real provider would.
    refund_replay_window: Some(Duration::from_secs(12 * 60 * 60)),
    charge_replay_window: Duration::from_secs(12 * 60 * 60),
    supports_3ds: true,
    supports_disputes: true,
    webhooks: Some(InboundWebhooks {
        requires_secret: false,
        // Mock synthesizes webhook registration, so the flag stays honest.
        self_registration: true,
        replay_tolerance_secs: NonZeroU32::new(300).unwrap(),
    }),
    asynchronous_settlement: true,
    // Mock stands in for webhook-driven providers in integration tests; hosted
    // checkouts complete without pending-intent persistence, as before.
    mandate_setup: MandateSetup::EmbeddedClientSecret,
    supports_hosted_checkout: true,
    pending_charge_accepted: false,
    is_test_gateway: true,
};

/// A provider transaction id the mock minted: `mock_pi_<amount>_<ccy>_<uuid>`. Reconciliation
/// has only the id to go on, so the charged amount rides in it.
#[derive(Debug, Clone)]
pub struct MockIntentRef {
    pub amount_minor: i64,
    pub currency: String,
}

impl MockIntentRef {
    fn mint(amount_minor: i64, currency: &str) -> String {
        format!("mock_pi_{amount_minor}_{currency}_{}", Uuid::now_v7())
    }

    pub fn parse(external_transaction_id: &str) -> Option<Self> {
        let rest = external_transaction_id.strip_prefix("mock_pi_")?;
        let mut parts = rest.split('_');
        let amount_minor = parts.next()?.parse::<i64>().ok()?;
        let currency = parts.next()?;
        if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        Some(Self {
            amount_minor,
            currency: currency.to_string(),
        })
    }
}

/// A refund id the mock minted: `mock_re_<amount>_<ccy>_<uuid>`, the same trick as
/// [`MockIntentRef`] so `fetch_refund` (the reconcile sweep) can answer without stored state.
#[derive(Debug, Clone)]
pub struct MockRefundRef {
    pub amount_minor: i64,
    pub currency: String,
}

impl MockRefundRef {
    fn mint(amount_minor: i64, currency: &str) -> String {
        format!("mock_re_{amount_minor}_{currency}_{}", Uuid::now_v7())
    }

    pub fn parse(external_refund_id: &str) -> Option<Self> {
        let rest = external_refund_id.strip_prefix("mock_re_")?;
        let mut parts = rest.split('_');
        let amount_minor = parts.next()?.parse::<i64>().ok()?;
        let currency = parts.next()?;
        if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        Some(Self {
            amount_minor,
            currency: currency.to_string(),
        })
    }
}

/// Marks a next-action as the mock's own: the frontend must complete it on the
/// mock challenge page instead of handing it to a payment SDK.
pub const MOCK_PUBLISHABLE_KEY: &str = "mock_pk_test_key";

/// Path of the challenge the frontend POSTs to for a `requires_action` charge.
/// Relative on purpose — the adapter has no business knowing the REST API's
/// external URL, and the frontend already has it.
fn challenge_path(connector: &ConnectorRef, intent_id: &str) -> String {
    format!(
        "/v1/portal/mock/3ds?tenant={}&connector={}&intent={}",
        connector.tenant_id.as_base62(),
        connector.id.as_base62(),
        intent_id
    )
}

#[derive(Debug, Clone)]
pub struct MockConnector {
    connector: ConnectorRef,
    config: MockPublicData,
}

impl MockConnector {
    pub fn new(connector: ConnectorRef, config: MockPublicData) -> Self {
        Self { connector, config }
    }
}

/// Charge outcome a mock scenario asks for, encoded in the payment method id
/// the checkout's mock form mints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockBehavior {
    Succeed,
    Decline,
    /// The card is past its expiry date: every charge is declined as expired.
    Expired,
    /// A hard decline (lost card): never worth retrying.
    HardDecline,
    /// Settles later, through a webhook the developer posts by hand.
    Pending,
    /// 3DS-style challenge; the charge comes back needing customer action,
    /// completed on the mock challenge page.
    RequiresAction,
    /// Challenges every charge, renewals included: an off-session charge is left awaiting
    /// authentication (`Pending` with a stored next action), as an issuer that re-challenges does.
    RequiresActionAlways,
    /// Transport failure: the outcome is UNCERTAIN, not a decline. Only
    /// reconciliation can tell whether the money moved.
    Unknown,
}

impl MockBehavior {
    fn code(self) -> &'static str {
        match self {
            Self::Succeed => "ok",
            Self::Decline => "decline",
            Self::Expired => "expired",
            Self::HardDecline => "hard",
            Self::Pending => "pending",
            Self::RequiresAction => "action",
            Self::RequiresActionAlways => "reauth",
            Self::Unknown => "unknown",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "ok" => Some(Self::Succeed),
            "decline" => Some(Self::Decline),
            "expired" => Some(Self::Expired),
            "hard" => Some(Self::HardDecline),
            "pending" => Some(Self::Pending),
            "action" => Some(Self::RequiresAction),
            "reauth" => Some(Self::RequiresActionAlways),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }

    /// The charge outcome this scenario asks for. `on_session` decides what a challenge means:
    /// nobody is present to answer one on a renewal.
    fn charge_behavior(self, on_session: bool) -> MockChargeBehavior {
        match self {
            Self::Succeed => MockChargeBehavior::Succeeded,
            Self::Decline => MockChargeBehavior::Failed,
            Self::Expired => MockChargeBehavior::Expired,
            Self::HardDecline => MockChargeBehavior::HardDeclined,
            Self::Pending => MockChargeBehavior::Pending,
            Self::RequiresAction if !on_session => MockChargeBehavior::Pending,
            Self::RequiresAction | Self::RequiresActionAlways => MockChargeBehavior::RequiresAction,
            Self::Unknown => MockChargeBehavior::Unknown,
        }
    }
}

/// `mock_pm_<behavior>_<type>_<hint>_<rand>`, minted from the fixed scenario catalog, never from
/// customer input. `<hint>` is `<brand>-<last4>` for cards, the account's last 4 otherwise.
/// Anything else (the ids tests use) parses to `None` and falls back to the connector config.
#[derive(Debug, Clone)]
pub struct MockPaymentMethodRef {
    pub behavior: MockBehavior,
    pub payment_method_type: PaymentMethodTypeEnum,
    pub card_brand: Option<String>,
    pub card_last4: Option<String>,
    pub account_number_hint: Option<String>,
}

impl MockPaymentMethodRef {
    /// Stable per scenario, so re-adding the same fake instrument dedupes like
    /// a real re-added card does.
    fn fingerprint(&self) -> String {
        let hint = match (
            &self.card_brand,
            &self.card_last4,
            &self.account_number_hint,
        ) {
            (Some(brand), Some(last4), _) => format!("{brand}-{last4}"),
            (_, _, Some(account)) => account.clone(),
            _ => "unknown".to_string(),
        };
        format!("mock_fp_{hint}")
    }

    pub fn parse(external_payment_method_id: &str) -> Option<Self> {
        let rest = external_payment_method_id.strip_prefix("mock_pm_")?;
        let mut parts = rest.split('_');
        let behavior = MockBehavior::parse(parts.next()?)?;
        let method = parts.next()?;
        let hint = parts.next()?;

        let is_last4 = |v: &str| v.len() == 4 && v.chars().all(|c| c.is_ascii_digit());

        if method == "card" {
            let (brand, last4) = hint.split_once('-')?;
            if brand.is_empty() || !is_last4(last4) {
                return None;
            }
            return Some(Self {
                behavior,
                payment_method_type: PaymentMethodTypeEnum::Card,
                card_brand: Some(brand.to_string()),
                card_last4: Some(last4.to_string()),
                account_number_hint: None,
            });
        }

        let payment_method_type = match method {
            "sepa" => PaymentMethodTypeEnum::DirectDebitSepa,
            "ach" => PaymentMethodTypeEnum::DirectDebitAch,
            "bacs" => PaymentMethodTypeEnum::DirectDebitBacs,
            _ => return None,
        };
        if !is_last4(hint) {
            return None;
        }
        Some(Self {
            behavior,
            payment_method_type,
            card_brand: None,
            card_last4: None,
            account_number_hint: Some(hint.to_string()),
        })
    }
}

impl ConnectorIdentity for MockConnector {
    fn provider(&self) -> PaymentProvider {
        PaymentProvider::Mock
    }

    fn capabilities(&self) -> &PaymentCapabilities {
        &MOCK_CAPABILITIES
    }
}

#[async_trait]
impl CredentialOps for MockConnector {
    async fn validate_credentials(&self) -> Result<ValidatedCredentials, Report<ConnectorError>> {
        let config = self.config.clone();
        if config.reject_credentials {
            return Err(Report::new(ConnectorError::Configuration(
                "mock credentials rejected (configured)".to_string(),
            ))
            .attach_opaque(super::error::CredentialsRejected));
        }
        let methods = MOCK_CAPABILITIES
            .supported_payment_methods
            .iter()
            .map(|method| PaymentMethodAvailability {
                method: method.clone(),
                state: if config.pending_methods.contains(method) {
                    MethodState::Pending
                } else {
                    MethodState::Enabled
                },
                reason: None,
            })
            .collect();
        Ok(ValidatedCredentials {
            account: ProviderAccount {
                account_id: Some(
                    config
                        .account_id
                        .clone()
                        .unwrap_or_else(|| "mock_account".to_string()),
                ),
                display_name: Some("Mock account".to_string()),
                livemode: false,
                key_hint: None,
                methods,
                checked_at: common_domain::clock::now().naive_utc(),
            },
            public: PaymentConnectorPublicData::Mock(config),
        })
    }
}

#[async_trait]
impl CustomerOps for MockConnector {
    async fn create_customer(
        &self,
        customer: &Customer,
        _request: CreateCustomerRequest,
    ) -> Result<ExternalCustomerRef, Report<ConnectorError>> {
        Ok(ExternalCustomerRef {
            external_id: format!("mock_cus_{}", customer.id.as_base62()),
            provider_request_id: None,
        })
    }
}

#[async_trait]
impl MandateOps for MockConnector {
    async fn initiate_mandate_setup(
        &self,
        _connection: &CustomerConnection,
        _request: MandateSetupRequest<'_>,
    ) -> Result<MandateSetupInstruction, Report<ConnectorError>> {
        if self.config.fail_setup_intent {
            return Err(Report::new(ConnectorError::MandateSetup(
                "mock setup intent failure (configured)".to_string(),
            )));
        }

        let intent_id = format!("mock_seti_{}", Uuid::now_v7());
        Ok(MandateSetupInstruction::EmbeddedClientSecret {
            intent_id,
            client_secret: format!("mock_secret_{}", Uuid::now_v7()),
            publishable_key: SecretString::from(MOCK_PUBLISHABLE_KEY.to_string()),
        })
    }

    async fn fetch_payment_method(
        &self,
        external_payment_method_id: &str,
        _external_customer_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        // Unrecognized ids keep the historical card snapshot the tests expect.
        let method = MockPaymentMethodRef::parse(external_payment_method_id);
        let is_card = method
            .as_ref()
            .is_none_or(|m| m.payment_method_type == PaymentMethodTypeEnum::Card);
        Ok(PaymentMethodSnapshot {
            external_payment_method_id: external_payment_method_id.to_string(),
            payment_method_type: method
                .as_ref()
                .map(|m| m.payment_method_type.clone())
                .unwrap_or(PaymentMethodTypeEnum::Card),
            account_number_hint: method.as_ref().and_then(|m| m.account_number_hint.clone()),
            card_brand: is_card.then(|| {
                method
                    .as_ref()
                    .and_then(|m| m.card_brand.clone())
                    .unwrap_or_else(|| "mock_visa".to_string())
            }),
            card_last4: is_card.then(|| {
                method
                    .as_ref()
                    .and_then(|m| m.card_last4.clone())
                    .unwrap_or_else(|| "4242".to_string())
            }),
            card_exp_month: is_card.then_some(12),
            card_exp_year: is_card.then_some(2030),
            fingerprint: Some(
                method
                    .as_ref()
                    .map(MockPaymentMethodRef::fingerprint)
                    .unwrap_or_else(|| "mock_fp_4242".to_string()),
            ),
            meteroid_connection_id: None,
            meteroid_customer_id: None,
            meteroid_invoice_id: None,
            meteroid_checkout_session_id: None,
            meteroid_transaction_id: None,
            payment_request_payment: None,
        })
    }

    async fn complete_mandate_setup(
        &self,
        intent_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        Ok(PaymentMethodSnapshot {
            external_payment_method_id: format!("mock_md_{intent_id}"),
            payment_method_type: PaymentMethodTypeEnum::DirectDebitSepa,
            account_number_hint: Some("0009".to_string()),
            card_brand: None,
            card_last4: None,
            card_exp_month: None,
            card_exp_year: None,
            fingerprint: Some("mock_fp_0009".to_string()),
            meteroid_connection_id: None,
            meteroid_customer_id: None,
            meteroid_invoice_id: None,
            meteroid_checkout_session_id: None,
            meteroid_transaction_id: None,
            payment_request_payment: None,
        })
    }
}

#[async_trait]
impl PaymentOps for MockConnector {
    async fn charge_off_session(
        &self,
        request: ChargeRequest<'_>,
    ) -> Result<ChargeOutcome, Report<ConnectorError>> {
        let external_id = MockIntentRef::mint(request.amount_minor, request.currency);

        // The connector config wins (that is how tests pin an outcome); with no
        // configured behavior, the scenario the customer picked decides.
        let behavior = if self.config.fail_payment_intent {
            MockChargeBehavior::Failed
        } else {
            match self.config.charge_behavior {
                Some(configured) => configured,
                None => MockPaymentMethodRef::parse(request.payment_method_external_id)
                    .map(|m| m.behavior.charge_behavior(request.on_session))
                    .unwrap_or(MockChargeBehavior::Succeeded),
            }
        };

        let outcome = match behavior {
            MockChargeBehavior::RejectedCredentials => {
                // As a provider answering 401 to a revoked key.
                return Err(Report::new(ConnectorError::Configuration(
                    "mock rejected the stored credentials (configured)".to_string(),
                ))
                .attach_opaque(super::error::CredentialsRejected));
            }
            // `Err` is the UNCERTAIN outcome: the caller must not record a decline,
            // and a retry reuses the same idempotency key.
            MockChargeBehavior::Unknown => {
                return Err(Report::new(ConnectorError::Transport(format!(
                    "mock transport failure; charge outcome unknown for {external_id}"
                ))));
            }
            MockChargeBehavior::Failed => ChargeOutcome::Failed(ChargeFailure {
                external_id: Some(external_id),
                code: Some("declined".to_string()),
                message: "Test gateway: payment declined".to_string(),
                retryable: false,
                decline_kind: DeclineKind::Other,
                provider_request_id: None,
            }),
            MockChargeBehavior::Expired => ChargeOutcome::Failed(ChargeFailure {
                external_id: Some(external_id),
                code: Some("expired_card".to_string()),
                message: "Test gateway: card expired".to_string(),
                retryable: false,
                decline_kind: DeclineKind::CardExpired,
                provider_request_id: None,
            }),
            MockChargeBehavior::HardDeclined => ChargeOutcome::Failed(ChargeFailure {
                external_id: Some(external_id),
                code: Some("lost_card".to_string()),
                message: "Test gateway: card declined (lost_card)".to_string(),
                retryable: false,
                decline_kind: DeclineKind::Fraud,
                provider_request_id: None,
            }),
            MockChargeBehavior::Cancelled => ChargeOutcome::Cancelled(ChargeCancelled {
                external_id: Some(external_id),
                message: "Mock payment cancelled (configured)".to_string(),
                provider_request_id: None,
            }),
            MockChargeBehavior::Pending => ChargeOutcome::Pending(ChargeAcknowledged {
                external_id,
                provider_request_id: None,
            }),
            MockChargeBehavior::RequiresAction => {
                // The "client secret" is the mock challenge path; the `mock_pk_test_key` marker
                // tells the frontend not to hand it to a real SDK.
                ChargeOutcome::RequiresAction(RequiresActionInstruction::ClientSecret {
                    client_secret: challenge_path(&self.connector, &external_id),
                    external_id,
                    publishable_key: SecretString::from(MOCK_PUBLISHABLE_KEY.to_string()),
                })
            }
            MockChargeBehavior::Succeeded => ChargeOutcome::Succeeded(ChargeReceipt {
                external_id,
                amount_received_minor: request.amount_minor,
                processed_at: common_domain::clock::now().naive_utc(),
                provider_request_id: None,
            }),
        };
        Ok(outcome)
    }

    /// The mock keeps no state: whether the charge still awaits its challenge is the caller's
    /// transaction to tell. Any intent the mock minted gets its challenge path back, unless a
    /// configured `reconcile_status` says the provider has moved on from the challenge.
    async fn fetch_pending_action(
        &self,
        external_transaction_id: &str,
    ) -> Result<Option<RequiresActionInstruction>, Report<ConnectorError>> {
        if self.config.reconcile_status.is_some() {
            return Ok(None);
        }
        Ok(MockIntentRef::parse(external_transaction_id).map(|_| {
            RequiresActionInstruction::ClientSecret {
                client_secret: challenge_path(&self.connector, external_transaction_id),
                external_id: external_transaction_id.to_string(),
                publishable_key: SecretString::from(MOCK_PUBLISHABLE_KEY.to_string()),
            }
        }))
    }
}

/// A top-up intent carries the scenario of the card it was prepared for, as
/// `mock_pi_<amount>_<ccy>_w<scenario>_<transaction>`: confirmation only sees the transaction.
fn wallet_intent_id(request: &PrepareWalletChargeRequest<'_>) -> String {
    let scenario = MockPaymentMethodRef::parse(request.payment_method_external_id)
        .map_or("ok", |method| method.behavior.code());
    format!(
        "mock_pi_{}_{}_w{scenario}_{}",
        request.amount_minor,
        request.currency,
        request.transaction_id.as_uuid()
    )
}

fn wallet_intent_scenario(external_id: &str) -> MockBehavior {
    external_id
        .split('_')
        .find_map(|part| part.strip_prefix('w').and_then(MockBehavior::parse))
        .unwrap_or(MockBehavior::Succeed)
}

#[async_trait]
impl WalletOps for MockConnector {
    async fn prepare_wallet_charge(
        &self,
        request: PrepareWalletChargeRequest<'_>,
    ) -> Result<String, Report<ConnectorError>> {
        record_mock_invocation(MockProviderInvocation {
            operation: MockProviderOperation::Prepare,
            tenant_id: self.connector.tenant_id,
            transaction_id: request.transaction_id,
            amount: request.amount_minor,
            currency: request.currency.to_owned(),
            captured: false,
        });
        Ok(wallet_intent_id(&request))
    }

    async fn resume_wallet_charge(
        &self,
        transaction: &PaymentTransaction,
        on_session: bool,
    ) -> Result<ChargeOutcome, Report<ConnectorError>> {
        let external_id = transaction.provider_transaction_id.clone().ok_or_else(|| {
            Report::new(ConnectorError::Charge(
                "a top-up payment is recorded before it is confirmed".to_owned(),
            ))
        })?;
        // Confirming an intent that already succeeded returns it; it never charges again.
        if mock_intent_captured(&external_id) {
            return Ok(ChargeOutcome::Succeeded(ChargeReceipt {
                external_id,
                amount_received_minor: transaction.amount,
                processed_at: common_domain::clock::now().naive_utc(),
                provider_request_id: None,
            }));
        }
        let behavior = if self.config.fail_payment_intent {
            MockChargeBehavior::Failed
        } else {
            self.config.charge_behavior.unwrap_or_else(|| {
                // The step itself is modelled here: off-session, a challenge cancels below.
                wallet_intent_scenario(&external_id).charge_behavior(true)
            })
        };
        let cancelled = |message: String| {
            ChargeOutcome::Cancelled(ChargeCancelled {
                external_id: Some(external_id.clone()),
                message,
                provider_request_id: None,
            })
        };
        let declined = |message: &str, decline_kind: DeclineKind| {
            ChargeOutcome::Failed(ChargeFailure {
                external_id: Some(external_id.clone()),
                code: None,
                message: message.to_owned(),
                retryable: false,
                decline_kind,
                provider_request_id: None,
            })
        };
        // A decline, or a step nobody can complete, cancels the intent rather than leave it live.
        let outcome = if transaction.status == crate::domain::PaymentStatusEnum::Cancelled {
            cancelled(
                transaction
                    .error_type
                    .clone()
                    .unwrap_or_else(|| "Test gateway: payment cancelled".to_owned()),
            )
        } else {
            match behavior {
                MockChargeBehavior::RejectedCredentials => {
                    return Err(Report::new(ConnectorError::Configuration(
                        "mock rejected the stored credentials (configured)".to_string(),
                    ))
                    .attach_opaque(super::error::CredentialsRejected));
                }
                MockChargeBehavior::Unknown => {
                    return Err(Report::new(ConnectorError::Transport(format!(
                        "mock transport failure; charge outcome unknown for {external_id}"
                    ))));
                }
                MockChargeBehavior::Failed => declined(
                    "Test gateway: payment declined (card_declined)",
                    DeclineKind::DoNotHonor,
                ),
                MockChargeBehavior::Expired => declined(
                    "Test gateway: card expired (expired_card)",
                    DeclineKind::CardExpired,
                ),
                MockChargeBehavior::HardDeclined => declined(
                    "Test gateway: card declined (lost_card)",
                    DeclineKind::Fraud,
                ),
                MockChargeBehavior::Cancelled => {
                    cancelled("Mock payment cancelled (configured)".to_owned())
                }
                MockChargeBehavior::RequiresAction if !on_session => declined(
                    "authentication_required: payment requires customer authentication",
                    DeclineKind::AuthenticationRequired,
                ),
                MockChargeBehavior::Pending => ChargeOutcome::Pending(ChargeAcknowledged {
                    external_id: external_id.clone(),
                    provider_request_id: None,
                }),
                MockChargeBehavior::RequiresAction => {
                    ChargeOutcome::RequiresAction(RequiresActionInstruction::ClientSecret {
                        client_secret: challenge_path(&self.connector, &external_id),
                        external_id: external_id.clone(),
                        publishable_key: SecretString::from(MOCK_PUBLISHABLE_KEY.to_string()),
                    })
                }
                MockChargeBehavior::Succeeded => ChargeOutcome::Succeeded(ChargeReceipt {
                    external_id: external_id.clone(),
                    amount_received_minor: transaction.amount,
                    processed_at: common_domain::clock::now().naive_utc(),
                    provider_request_id: None,
                }),
            }
        };
        let captured =
            matches!(outcome, ChargeOutcome::Succeeded(_)) && mock_capture_is_new(&external_id);
        record_mock_invocation(MockProviderInvocation {
            operation: MockProviderOperation::Resume,
            tenant_id: self.connector.tenant_id,
            transaction_id: transaction.id,
            amount: transaction.amount,
            currency: transaction.currency.clone(),
            captured,
        });
        Ok(outcome)
    }
}

/// Which wallet entry point a mock invocation came through. Only `Resume` can move money.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockProviderOperation {
    Prepare,
    Resume,
}

/// One recorded wallet call to the mock, so tests can tell a replay from a second charge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockProviderInvocation {
    pub operation: MockProviderOperation,
    pub tenant_id: common_domain::ids::TenantId,
    pub transaction_id: common_domain::ids::PaymentTransactionId,
    pub amount: i64,
    pub currency: String,
    pub captured: bool,
}

// Per thread: each test drives its server and workers on its own current-thread runtime.
// Off unless a test switches it on, so a deployed mock provider accumulates nothing.
thread_local! {
    static MOCK_RECORDING: std::cell::Cell<bool> = const { std::cell::Cell::new(cfg!(test)) };
    static MOCK_PROVIDER_INVOCATIONS: std::cell::RefCell<Vec<MockProviderInvocation>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static MOCK_CAPTURED_INTENTS: std::cell::RefCell<std::collections::BTreeSet<String>> =
        const { std::cell::RefCell::new(std::collections::BTreeSet::new()) };
}

fn mock_recording() -> bool {
    MOCK_RECORDING.get()
}

fn record_mock_invocation(invocation: MockProviderInvocation) {
    if mock_recording() {
        MOCK_PROVIDER_INVOCATIONS.with_borrow_mut(|log| log.push(invocation));
    }
}

fn mock_capture_is_new(external_id: &str) -> bool {
    !mock_recording()
        || MOCK_CAPTURED_INTENTS.with_borrow_mut(|captured| captured.insert(external_id.to_owned()))
}

fn mock_intent_captured(external_id: &str) -> bool {
    mock_recording() && MOCK_CAPTURED_INTENTS.with_borrow(|captured| captured.contains(external_id))
}

pub fn mock_provider_invocations() -> Vec<MockProviderInvocation> {
    MOCK_PROVIDER_INVOCATIONS.with_borrow(|log| log.clone())
}

/// Clears this thread's log and starts recording on it.
pub fn reset_mock_provider_invocations() {
    MOCK_RECORDING.set(true);
    MOCK_PROVIDER_INVOCATIONS.with_borrow_mut(Vec::clear);
    MOCK_CAPTURED_INTENTS.with_borrow_mut(std::collections::BTreeSet::clear);
}

#[async_trait]
impl RefundOps for MockConnector {
    async fn refund(
        &self,
        request: RefundRequest<'_>,
    ) -> Result<RefundOutcome, Report<ConnectorError>> {
        let external_refund_id = MockRefundRef::mint(request.amount_minor, request.currency);

        Ok(match self.config.refund_behavior {
            Some(MockRefundBehavior::Failed) => RefundOutcome::Failed(RefundFailure {
                code: Some("refund_declined".to_string()),
                message: "Mock refund declined".to_string(),
                provider_request_id: None,
            }),
            Some(MockRefundBehavior::Pending) => RefundOutcome::Pending(RefundAcknowledged {
                external_refund_id,
                provider_request_id: None,
            }),
            Some(MockRefundBehavior::Succeeded) | None => RefundOutcome::Succeeded(RefundReceipt {
                external_refund_id,
                amount_refunded_minor: request.amount_minor,
                processed_at: common_domain::clock::now().naive_utc(),
                provider_request_id: None,
            }),
        })
    }

    /// Backs the reconcile sweep for a refund the mock left `Pending`: any `mock_re_…` id it
    /// minted resolves as settled, the amount read back from the id itself (see
    /// [`MockRefundRef`]). An id it never minted is genuinely unknown to it.
    async fn fetch_refund(
        &self,
        external_refund_id: &str,
        _external_transaction_id: Option<&str>,
    ) -> Result<RefundSnapshot, Report<ConnectorError>> {
        let Some(refund) = MockRefundRef::parse(external_refund_id) else {
            return Err(Report::new(ConnectorError::Unsupported {
                provider: self.provider().into(),
                capability: "fetch_refund",
            }));
        };
        Ok(RefundSnapshot {
            external_transaction_id: String::new(),
            external_refund_id: external_refund_id.to_string(),
            amount_minor: refund.amount_minor,
            status: RemoteRefundStatus::Succeeded,
            cumulative_refunded_minor: None,
            currency: refund.currency,
        })
    }
}

#[async_trait]
impl ReconcileOps for MockConnector {
    /// What the provider tells the reconciliation sweep. Unset means "still in
    /// flight", so a mock connector never resolves a stuck payment by accident.
    async fn fetch_transaction_status(
        &self,
        external_transaction_id: &str,
    ) -> Result<RemoteTransactionStatus, Report<ConnectorError>> {
        Ok(match self.config.reconcile_status {
            // The settled amount comes from the id the mock itself minted; an
            // id it never minted is genuinely unknown to it.
            Some(MockReconcileStatus::Succeeded) => {
                match MockIntentRef::parse(external_transaction_id) {
                    Some(intent) => RemoteTransactionStatus::Succeeded {
                        amount_received_minor: intent.amount_minor,
                        currency: intent.currency,
                        processed_at: common_domain::clock::now().naive_utc(),
                    },
                    None => RemoteTransactionStatus::Unknown,
                }
            }
            Some(MockReconcileStatus::Failed) => RemoteTransactionStatus::Failed {
                code: Some("mock_reconciled_failure".to_string()),
                message: "Mock provider reports the charge failed".to_string(),
                decline_kind: DeclineKind::Other,
            },
            Some(MockReconcileStatus::Cancelled) => RemoteTransactionStatus::Cancelled,
            Some(MockReconcileStatus::Unknown) => RemoteTransactionStatus::Unknown,
            Some(MockReconcileStatus::Processing) => RemoteTransactionStatus::Processing,
            Some(MockReconcileStatus::Pending) | None => RemoteTransactionStatus::Pending,
        })
    }
}

#[async_trait]
impl WebhookOps for MockConnector {
    async fn register_webhook(
        &self,
        _url: &str,
        _subscriptions: &[NormalizedEventSubscription],
    ) -> Result<RegisteredWebhook, Report<ConnectorError>> {
        Ok(RegisteredWebhook {
            endpoint_id: format!("mock_we_{}", Uuid::now_v7()),
            secret: SecretString::from(format!("mock_whsec_{}", Uuid::now_v7())),
        })
    }

    async fn unregister_webhook(&self, _endpoint_id: &str) -> Result<(), Report<ConnectorError>> {
        Ok(())
    }

    async fn sync_webhook_events(
        &self,
        _endpoint_id: &str,
        _subscriptions: &[NormalizedEventSubscription],
    ) -> Result<(), Report<ConnectorError>> {
        Ok(())
    }

    /// Unreachable in practice: the mock holds no webhook secret, so the inbound route refuses
    /// it before verification.
    fn verify_signature(
        &self,
        _payload: &[u8],
        _headers: &HeaderMap,
        _secret: &SecretString,
    ) -> Result<(), Report<ConnectorError>> {
        Ok(())
    }

    /// Tests drive webhook flows by POSTing a small JSON envelope (see
    /// [`MockWebhookEvent`]); we map it straight to a normalized event. A real
    /// provider would parse its own wire format here.
    fn parse_event(
        &self,
        payload: &[u8],
        _headers: &HeaderMap,
    ) -> Result<Option<NormalizedWebhookEvent>, Report<ConnectorError>> {
        if payload.is_empty() {
            return Ok(None);
        }
        let e: MockWebhookEvent = serde_json::from_slice(payload).map_err(|err| {
            Report::new(ConnectorError::PayloadDecode(format!(
                "mock webhook decode: {err}"
            )))
        })?;

        let kind = match e.kind.as_str() {
            "payment_succeeded" => NormalizedEventKind::PaymentSucceeded(PaymentSucceededEvent {
                external_transaction_id: e.external_id.clone().unwrap_or_default(),
                amount_received_minor: e.amount.unwrap_or(0),
                currency: e.currency.clone().unwrap_or_default(),
                meteroid_transaction_id: e.transaction_id.clone(),
            }),
            "payment_failed" => NormalizedEventKind::PaymentFailed(PaymentFailedEvent {
                external_transaction_id: e.external_id.clone().unwrap_or_default(),
                code: Some("mock_failed".into()),
                message: "mock payment failed".into(),
                retryable: false,
                decline_kind: DeclineKind::Other,
                meteroid_transaction_id: e.transaction_id.clone(),
            }),
            "payment_pending" => NormalizedEventKind::PaymentPending(PaymentPendingEvent {
                external_transaction_id: e.external_id.clone().unwrap_or_default(),
                meteroid_transaction_id: e.transaction_id.clone(),
                awaiting_customer: false,
            }),
            // Post-settlement reversals: `amount` carries the cumulative refunded
            // total for `payment_refunded` (partial-aware); the others are full.
            "payment_refunded" => NormalizedEventKind::PaymentRefunded(PaymentRefundedEvent {
                external_transaction_id: e.external_id.clone().unwrap_or_default(),
                external_refund_id: format!("mock_re_{}", e.id),
                amount_refunded_minor: e.amount.unwrap_or(0),
                currency: e.currency.clone().unwrap_or_default(),
                refunds: e
                    .refunds
                    .iter()
                    .map(|r| ObservedRefundOnWire {
                        external_refund_id: r.id.clone(),
                        amount_minor: r.amount,
                        status: if r.pending {
                            ObservedRefundStatus::Pending
                        } else if r.succeeded {
                            ObservedRefundStatus::Succeeded
                        } else {
                            ObservedRefundStatus::Failed
                        },
                    })
                    .collect(),
            }),
            "payment_reversed" => NormalizedEventKind::PaymentReversed(PaymentReversedEvent {
                external_transaction_id: e.external_id.clone().unwrap_or_default(),
                meteroid_transaction_id: e.transaction_id.clone(),
                reason: "mock_reversed".into(),
                kind: ReversalKind::Other,
                disputed: false,
                cause: None,
            }),
            // Bank-side claw-back (GoCardless `late_failure_settled`, `insufficient_funds`).
            "payment_late_failure" => NormalizedEventKind::PaymentReversed(PaymentReversedEvent {
                external_transaction_id: e.external_id.clone().unwrap_or_default(),
                meteroid_transaction_id: e.transaction_id.clone(),
                reason: "late_failure_settled".into(),
                kind: ReversalKind::InsufficientFunds,
                disputed: false,
                cause: Some("insufficient_funds".into()),
            }),
            // Full clawback (GoCardless `charged_back` shape): always the whole amount, the
            // payer's claim, so the invoice stays paid.
            "payment_charged_back" => NormalizedEventKind::PaymentReversed(PaymentReversedEvent {
                external_transaction_id: e.external_id.clone().unwrap_or_default(),
                meteroid_transaction_id: e.transaction_id.clone(),
                reason: "mock_chargeback".into(),
                kind: ReversalKind::Chargeback,
                disputed: true,
                cause: Some("mock_chargeback".into()),
            }),
            // Full-amount reinstatement (GoCardless `chargeback_cancelled` shape).
            "payment_reinstated" => {
                NormalizedEventKind::PaymentReinstated(PaymentReinstatedEvent {
                    external_transaction_id: e.external_id.clone().unwrap_or_default(),
                    meteroid_transaction_id: e.transaction_id.clone(),
                    reason: "mock_reinstated".into(),
                })
            }
            // Stripe shape: `reason` classifies a bank return reported as a dispute.
            "dispute_funds_withdrawn" => NormalizedEventKind::DisputeFundsWithdrawn(DisputeEvent {
                external_dispute_id: mock_dispute_id(&e),
                external_transaction_id: e.external_id.clone().unwrap_or_default(),
                amount_minor: e.amount.unwrap_or(0),
                currency: e.currency.clone().unwrap_or_default(),
                failure: e.reason.as_deref().and_then(super::stripe::dispute_failure),
                reason: Some(e.reason.clone().unwrap_or_else(|| "mock_dispute".into())),
            }),
            "dispute_funds_reinstated" => {
                NormalizedEventKind::DisputeFundsReinstated(DisputeEvent {
                    external_dispute_id: mock_dispute_id(&e),
                    external_transaction_id: e.external_id.clone().unwrap_or_default(),
                    amount_minor: e.amount.unwrap_or(0),
                    currency: e.currency.clone().unwrap_or_default(),
                    reason: Some("mock_dispute".into()),
                    failure: None,
                })
            }
            "payment_requires_action" => {
                NormalizedEventKind::PaymentRequiresAction(PaymentRequiresActionEvent {
                    external_transaction_id: e.external_id.clone().unwrap_or_default(),
                    action_url: e.action_url.clone(),
                    client_secret: e.client_secret.clone(),
                    meteroid_transaction_id: e.transaction_id.clone(),
                })
            }
            "payment_method_attached" => {
                NormalizedEventKind::PaymentMethodAttached(PaymentMethodAttachedEvent {
                    external_customer_id: e.external_customer_id.clone().unwrap_or_default(),
                    external_payment_method_id: e
                        .external_payment_method_id
                        .clone()
                        .unwrap_or_default(),
                    payment_method_type: PaymentMethodTypeEnum::Card,
                    meteroid_connection_id: e.connection_id.clone(),
                    meteroid_customer_id: e.customer_id.clone(),
                    purpose: e
                        .setup_purpose
                        .as_deref()
                        .and_then(super::model::SetupPurpose::parse),
                })
            }
            "payment_method_updated" => {
                NormalizedEventKind::PaymentMethodUpdated(PaymentMethodUpdatedEvent {
                    external_payment_method_id: e
                        .external_payment_method_id
                        .clone()
                        .unwrap_or_default(),
                    card_brand: e.card_brand.clone(),
                    card_last4: e.card_last4.clone(),
                    card_exp_month: e.card_exp_month,
                    card_exp_year: e.card_exp_year,
                })
            }
            _ => NormalizedEventKind::Acknowledged {
                reason: "mock unhandled event kind",
            },
        };

        Ok(Some(NormalizedWebhookEvent {
            provider_event_id: e.id,
            provider_event_type: e.kind,
            // Tests pin `occurred_at` to exercise ordering/redelivery guards;
            // a real provider always stamps the event's own creation time.
            occurred_at: e.occurred_at.unwrap_or_else(common_domain::clock::now),
            kind,
            owner_tenant_id: None,
        }))
    }
}

/// Minimal webhook envelope the mock understands — tests post this JSON to
/// exercise the webhook → settlement path without a real provider.
#[derive(serde::Deserialize)]
struct MockWebhookEvent {
    id: String,
    kind: String,
    #[serde(default)]
    transaction_id: Option<String>,
    #[serde(default)]
    external_id: Option<String>,
    #[serde(default)]
    external_payment_method_id: Option<String>,
    #[serde(default)]
    external_customer_id: Option<String>,
    #[serde(default)]
    connection_id: Option<String>,
    #[serde(default)]
    customer_id: Option<String>,
    /// `payment_method_attached`: the setup intent's `meteroid.setup_purpose`, as Stripe carries it.
    #[serde(default)]
    setup_purpose: Option<String>,
    #[serde(default)]
    amount: Option<i64>,
    #[serde(default)]
    currency: Option<String>,
    /// Individual refunds behind a `payment_refunded` envelope's cumulative `amount`
    /// (Stripe `refunds.data[]` shape); the mock only ever carries one.
    #[serde(default)]
    refunds: Vec<MockRefundEntry>,
    #[serde(default)]
    occurred_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    action_url: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default)]
    card_brand: Option<String>,
    #[serde(default)]
    card_last4: Option<String>,
    #[serde(default)]
    card_exp_month: Option<i32>,
    #[serde(default)]
    card_exp_year: Option<i32>,
    /// A dispute's reason (Stripe vocabulary).
    #[serde(default)]
    reason: Option<String>,
    /// The dispute both its withdrawal and its reinstatement name, as a provider's dispute id.
    #[serde(default)]
    dispute_id: Option<String>,
}

fn mock_dispute_id(e: &MockWebhookEvent) -> String {
    e.dispute_id
        .clone()
        .unwrap_or_else(|| format!("mock_dp_{}", e.external_id.clone().unwrap_or_default()))
}

#[derive(serde::Deserialize)]
struct MockRefundEntry {
    id: String,
    #[serde(default)]
    amount: Option<i64>,
    #[serde(default)]
    succeeded: bool,
    #[serde(default)]
    pending: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::connectors::{
        Connector, ConnectorConfig, CredentialState, MockSensitiveData, PaymentConnectorConfig,
        PaymentProviderConfig,
    };
    use common_domain::ids::{ConnectorId, TenantId};
    use secrecy::ExposeSecret;

    fn connector_with(data: MockPublicData) -> Connector {
        Connector {
            status: crate::domain::enums::ConnectorStatusEnum::Active,
            credentials: CredentialState::Unverified,
            webhook_route: None,
            id: ConnectorId::new(),
            created_at: chrono::NaiveDateTime::default(),
            tenant_id: TenantId::new(),
            alias: "mock".into(),
            config: ConnectorConfig::Payment(PaymentConnectorConfig::configured(
                PaymentProviderConfig::Mock(data, MockSensitiveData {}),
            )),
        }
    }

    fn adapter(connector: &Connector) -> MockConnector {
        let config = match connector.payment_credentials() {
            Some(PaymentProviderConfig::Mock(d, _)) => d.clone(),
            _ => MockPublicData::default(),
        };
        MockConnector::new(ConnectorRef::from(connector), config)
    }

    fn connector(behavior: Option<MockChargeBehavior>) -> Connector {
        connector_with(MockPublicData {
            charge_behavior: behavior,
            ..Default::default()
        })
    }

    fn charge_request(payment_method_external_id: &str) -> ChargeRequest<'_> {
        ChargeRequest {
            descriptor: None,
            webhook_url: None,
            transaction_id: common_domain::ids::PaymentTransactionId::new(),
            customer_external_id: "cus",
            payment_method_external_id,
            payment_method_type: PaymentMethodTypeEnum::Card,
            amount_minor: 500,
            currency: "EUR",
            idempotency_key: super::super::model::IdempotencyKey::new("k"),
            on_session: true,
        }
    }

    async fn charge(behavior: Option<MockChargeBehavior>) -> ChargeOutcome {
        let c = connector(behavior);
        adapter(&c)
            .charge_off_session(charge_request("pm"))
            .await
            .unwrap()
    }

    async fn prepared(
        mock: &MockConnector,
        card: &str,
    ) -> crate::domain::payment_transactions::PaymentTransaction {
        let id = common_domain::ids::PaymentTransactionId::new();
        let intent = mock
            .prepare_wallet_charge(PrepareWalletChargeRequest {
                transaction_id: id,
                customer_external_id: "cus",
                payment_method_external_id: card,
                payment_method_type: PaymentMethodTypeEnum::Card,
                amount_minor: 500,
                currency: "EUR",
                idempotency_key: super::super::model::IdempotencyKey::new("k"),
            })
            .await
            .unwrap();
        crate::domain::payment_transactions::PaymentTransaction {
            id,
            tenant_id: mock.connector.tenant_id,
            invoice_id: None,
            provider_transaction_id: Some(intent),
            processed_at: None,
            refunded_at: None,
            amount: 500,
            currency: "EUR".into(),
            payment_method_id: None,
            status: crate::domain::PaymentStatusEnum::Pending,
            payment_type: crate::domain::PaymentTypeEnum::TopUp,
            error_type: None,
            receipt_pdf_id: None,
            checkout_session_id: None,
            pending_plan_version_id: None,
            pending_provider_intent_id: None,
            pending_connection_id: None,
            amount_refunded: 0,
            amount_reversed: 0,
            amount_disputed: 0,
            amount_reversed_excess: 0,
            reversal_kind: None,
            reversal_reason: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            next_action: None,
            decline_kind: None,
        }
    }

    /// A prepared top-up keeps its card's scenario, is captured once however often it is
    /// confirmed, and a withdrawn or unanswerable one is cancelled instead of charged.
    #[tokio::test]
    async fn a_prepared_top_up_is_confirmed_at_most_once() {
        let c = connector(None);
        let mock = adapter(&c);
        let payment = prepared(&mock, "mock_pm_ok_card_visa-4242_x").await;
        assert!(
            MockIntentRef::parse(payment.provider_transaction_id.as_deref().unwrap()).is_some()
        );
        for _ in 0..2 {
            assert!(matches!(
                mock.resume_wallet_charge(&payment, false).await.unwrap(),
                ChargeOutcome::Succeeded(_)
            ));
        }
        let captured = mock_provider_invocations()
            .iter()
            .filter(|call| call.transaction_id == payment.id && call.captured)
            .count();
        assert_eq!(captured, 1);

        let challenged = prepared(&mock, "mock_pm_action_card_visa-4242_x").await;
        assert!(matches!(
            mock.resume_wallet_charge(&challenged, true).await.unwrap(),
            ChargeOutcome::RequiresAction(_)
        ));
        assert!(matches!(
            mock.resume_wallet_charge(&challenged, false).await.unwrap(),
            ChargeOutcome::Failed(ChargeFailure {
                decline_kind: DeclineKind::AuthenticationRequired,
                ..
            })
        ));
        let mut withdrawn = prepared(&mock, "mock_pm_ok_card_visa-4242_x").await;
        withdrawn.status = crate::domain::PaymentStatusEnum::Cancelled;
        assert!(matches!(
            mock.resume_wallet_charge(&withdrawn, false).await.unwrap(),
            ChargeOutcome::Cancelled(_)
        ));
    }

    /// The mock is the reference adapter, so it must pass the contract.
    #[tokio::test]
    async fn mock_satisfies_contract() {
        let c = connector(None);
        super::super::contract::run_contract(&adapter(&c), &c).await;
    }

    #[tokio::test]
    async fn charge_behaviors() {
        assert!(matches!(charge(None).await, ChargeOutcome::Succeeded(_)));
        assert!(matches!(
            charge(Some(MockChargeBehavior::Pending)).await,
            ChargeOutcome::Pending(_)
        ));
        assert!(matches!(
            charge(Some(MockChargeBehavior::Failed)).await,
            ChargeOutcome::Failed(_)
        ));
        assert!(matches!(
            charge(Some(MockChargeBehavior::RequiresAction)).await,
            ChargeOutcome::RequiresAction(_)
        ));
    }

    async fn charge_with_method(external_payment_method_id: &str) -> ChargeOutcome {
        let c = connector(None);
        adapter(&c)
            .charge_off_session(charge_request(external_payment_method_id))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_card_drives_charge_outcome() {
        assert!(matches!(
            charge_with_method("mock_pm_ok_card_visa-4242_abc").await,
            ChargeOutcome::Succeeded(_)
        ));
        assert!(matches!(
            charge_with_method("mock_pm_decline_card_visa-0002_abc").await,
            ChargeOutcome::Failed(_)
        ));
        assert!(matches!(
            charge_with_method("mock_pm_pending_sepa_0009_abc").await,
            ChargeOutcome::Pending(_)
        ));
        // An id that isn't a mock card keeps the default success outcome.
        assert!(matches!(
            charge_with_method("pm_whatever").await,
            ChargeOutcome::Succeeded(_)
        ));
    }

    #[tokio::test]
    async fn expired_and_lost_cards_decline_hard_on_every_path() {
        for (card, kind) in [
            (
                "mock_pm_expired_card_mastercard-4444_abc",
                DeclineKind::CardExpired,
            ),
            ("mock_pm_hard_card_visa-0002_abc", DeclineKind::Fraud),
        ] {
            let ChargeOutcome::Failed(failure) = charge_with_method(card).await else {
                panic!("{card} is declined");
            };
            assert_eq!(failure.decline_kind, kind);
            assert!(failure.decline_kind.is_hard());

            let c = connector(None);
            let mock = adapter(&c);
            let transaction = prepared(&mock, card).await;
            let ChargeOutcome::Failed(failure) = mock
                .resume_wallet_charge(&transaction, false)
                .await
                .unwrap()
            else {
                panic!("a top-up on {card} is declined");
            };
            assert_eq!(failure.decline_kind, kind);
        }
    }

    /// The challenge next-action carries the mock's own completion path, not a
    /// client secret an SDK could consume.
    #[tokio::test]
    async fn requires_action_points_at_the_mock_challenge() {
        let outcome = charge_with_method("mock_pm_action_card_visa-3220_abc").await;
        let ChargeOutcome::RequiresAction(RequiresActionInstruction::ClientSecret {
            client_secret,
            publishable_key,
            external_id,
        }) = outcome
        else {
            panic!("expected a client-secret action");
        };
        // Every id the handler needs rides in the query string.
        assert!(client_secret.starts_with("/v1/portal/mock/3ds?tenant="));
        assert!(client_secret.contains("&connector="));
        assert!(client_secret.contains(&format!("&intent={external_id}")));
        assert_eq!(publishable_key.expose_secret(), MOCK_PUBLISHABLE_KEY);
    }

    #[tokio::test]
    async fn configured_behavior_beats_the_test_card() {
        let c = connector(Some(MockChargeBehavior::Pending));
        let outcome = adapter(&c)
            .charge_off_session(charge_request("mock_pm_ok_card_visa-4242_abc"))
            .await
            .unwrap();
        assert!(matches!(outcome, ChargeOutcome::Pending(_)));
    }

    /// An uncertain charge is an `Err`, never `Ok(Failed)`: the caller must not
    /// record a decline for a charge that may well have taken the money.
    #[tokio::test]
    async fn unknown_behavior_is_an_error_not_a_decline() {
        let c = connector(Some(MockChargeBehavior::Unknown));
        let err = adapter(&c)
            .charge_off_session(charge_request("pm"))
            .await
            .expect_err("an uncertain charge must not resolve to an outcome");
        assert!(matches!(
            err.current_context(),
            ConnectorError::Transport(_)
        ));

        let by_card = adapter(&connector(None))
            .charge_off_session(charge_request("mock_pm_unknown_card_visa-0127_a"))
            .await;
        assert!(by_card.is_err());
    }

    /// Reconciliation only has the provider id to go on, so the charged amount
    /// rides in it — otherwise a settled amount would be invented.
    #[tokio::test]
    async fn intent_id_carries_the_charged_amount() {
        let ChargeOutcome::Succeeded(receipt) = charge(None).await else {
            panic!("expected a settled charge");
        };
        let parsed = MockIntentRef::parse(&receipt.external_id).expect("minted id parses");
        assert_eq!(parsed.amount_minor, 500);
        assert_eq!(parsed.currency, "EUR");
    }

    async fn reconcile(
        status: Option<MockReconcileStatus>,
        external_id: &str,
    ) -> RemoteTransactionStatus {
        let c = connector_with(MockPublicData {
            reconcile_status: status,
            ..Default::default()
        });
        adapter(&c)
            .fetch_transaction_status(external_id)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn reconcile_status_drives_the_sweep() {
        let minted = "mock_pi_500_EUR_0199c0ab";

        assert!(matches!(
            reconcile(None, minted).await,
            RemoteTransactionStatus::Pending
        ));
        assert!(matches!(
            reconcile(Some(MockReconcileStatus::Failed), minted).await,
            RemoteTransactionStatus::Failed { .. }
        ));
        assert!(matches!(
            reconcile(Some(MockReconcileStatus::Cancelled), minted).await,
            RemoteTransactionStatus::Cancelled
        ));
        assert!(matches!(
            reconcile(Some(MockReconcileStatus::Unknown), minted).await,
            RemoteTransactionStatus::Unknown
        ));

        match reconcile(Some(MockReconcileStatus::Succeeded), minted).await {
            RemoteTransactionStatus::Succeeded {
                amount_received_minor,
                currency,
                ..
            } => {
                assert_eq!(amount_received_minor, 500);
                assert_eq!(currency, "EUR");
            }
            other => panic!("expected Succeeded, got {other:?}"),
        }

        // An id the mock never minted is genuinely unknown to it, whatever the
        // configured status — it cannot report an amount it never charged.
        assert!(matches!(
            reconcile(
                Some(MockReconcileStatus::Succeeded),
                "pi_from_another_provider"
            )
            .await,
            RemoteTransactionStatus::Unknown
        ));
    }

    #[tokio::test]
    async fn refund_behavior_drives_the_outcome() {
        let refund = async |behavior: Option<MockRefundBehavior>| {
            let c = connector_with(MockPublicData {
                refund_behavior: behavior,
                ..Default::default()
            });
            adapter(&c)
                .refund(RefundRequest {
                    external_transaction_id: "mock_pi_500_EUR_0199c0ab",
                    amount_minor: 500,
                    currency: "EUR",
                    reason: None,
                    metadata: Vec::new(),
                    idempotency_key: super::super::model::IdempotencyKey::new("k"),
                })
                .await
                .unwrap()
        };

        assert!(matches!(refund(None).await, RefundOutcome::Succeeded(_)));
        assert!(matches!(
            refund(Some(MockRefundBehavior::Pending)).await,
            RefundOutcome::Pending(_)
        ));
        assert!(matches!(
            refund(Some(MockRefundBehavior::Failed)).await,
            RefundOutcome::Failed(_)
        ));
    }

    #[tokio::test]
    async fn fetch_payment_method_reads_the_method_from_its_id() {
        let c = connector(None);
        let mock = adapter(&c);

        let card = mock
            .fetch_payment_method("mock_pm_decline_card_mastercard-0002_abc", "cus")
            .await
            .unwrap();
        assert_eq!(card.payment_method_type, PaymentMethodTypeEnum::Card);
        assert_eq!(card.card_brand.as_deref(), Some("mastercard"));
        assert_eq!(card.card_last4.as_deref(), Some("0002"));

        let sepa = mock
            .fetch_payment_method("mock_pm_ok_sepa_0009_abc", "cus")
            .await
            .unwrap();
        assert_eq!(
            sepa.payment_method_type,
            PaymentMethodTypeEnum::DirectDebitSepa
        );
        assert_eq!(sepa.account_number_hint.as_deref(), Some("0009"));
        assert!(sepa.card_brand.is_none());
        assert!(sepa.card_last4.is_none());
        assert!(sepa.card_exp_month.is_none());

        let fallback = mock
            .fetch_payment_method("pm_whatever", "cus")
            .await
            .unwrap();
        assert_eq!(fallback.card_last4.as_deref(), Some("4242"));
    }

    /// A recurring charge cannot show the customer a challenge, so the 3DS scenario behaves as an
    /// in-flight payment the merchant settles from the backoffice.
    #[tokio::test]
    async fn off_session_challenge_scenario_is_pending() {
        let c = connector(None);
        let mut request = charge_request("mock_pm_action_card_visa-3220_abc");
        request.on_session = false;
        let outcome = adapter(&c).charge_off_session(request).await.unwrap();
        assert!(matches!(outcome, ChargeOutcome::Pending(_)));
    }

    /// The "3DS on every charge" scenario challenges a renewal too: the charge is left awaiting
    /// authentication instead of plain pending.
    #[tokio::test]
    async fn off_session_reauth_scenario_requires_action() {
        let c = connector(None);
        let mut request = charge_request("mock_pm_reauth_card_visa-3184_abc");
        request.on_session = false;
        let outcome = adapter(&c).charge_off_session(request).await.unwrap();
        assert!(matches!(outcome, ChargeOutcome::RequiresAction(_)));
    }

    #[test]
    fn payment_method_ref_rejects_malformed_ids() {
        assert!(MockPaymentMethodRef::parse("mock_pm_ok_card_visa-4242_abc").is_some());
        assert!(MockPaymentMethodRef::parse("mock_pm_ok_bacs_0009_abc").is_some());
        assert!(MockPaymentMethodRef::parse("mock_pm_unknown_card_visa-0127_a").is_some());
        assert!(MockPaymentMethodRef::parse("mock_pm_explode_card_visa-4242_abc").is_none());
        assert!(MockPaymentMethodRef::parse("mock_pm_ok_card_visa-42_abc").is_none());
        assert!(MockPaymentMethodRef::parse("mock_pm_ok_wire_0009_abc").is_none());
        assert!(MockPaymentMethodRef::parse("mock_seti_123").is_none());
    }

    #[test]
    fn parse_event_maps_kinds() {
        let c = connector(None);
        let mock = adapter(&c);
        let payload = br#"{"id":"ev1","kind":"payment_succeeded","external_id":"pi_1","transaction_id":"tx_1","amount":500,"currency":"EUR"}"#;
        let ev = mock
            .parse_event(payload, &HeaderMap::new())
            .unwrap()
            .unwrap();
        match ev.kind {
            NormalizedEventKind::PaymentSucceeded(e) => {
                assert_eq!(e.meteroid_transaction_id.as_deref(), Some("tx_1"));
                assert_eq!(e.amount_received_minor, 500);
            }
            other => panic!("expected PaymentSucceeded, got {other:?}"),
        }

        let ra = br#"{"id":"ev2","kind":"payment_requires_action","external_id":"pi_2","transaction_id":"tx_2","client_secret":"pi_2_secret"}"#;
        let ev = mock.parse_event(ra, &HeaderMap::new()).unwrap().unwrap();
        assert!(matches!(
            ev.kind,
            NormalizedEventKind::PaymentRequiresAction(_)
        ));
    }

    #[test]
    fn parse_event_carries_the_refunds_on_a_payment_refunded() {
        let c = connector(None);
        let mock = adapter(&c);
        let payload = br#"{"id":"ev3","kind":"payment_refunded","external_id":"pi_3","amount":300,"currency":"EUR","refunds":[{"id":"mock_re_1","amount":300,"succeeded":true}]}"#;
        let ev = mock
            .parse_event(payload, &HeaderMap::new())
            .unwrap()
            .unwrap();
        match ev.kind {
            NormalizedEventKind::PaymentRefunded(e) => {
                assert_eq!(e.amount_refunded_minor, 300);
                assert_eq!(e.refunds.len(), 1);
                assert_eq!(e.refunds[0].external_refund_id, "mock_re_1");
                assert_eq!(e.refunds[0].amount_minor, Some(300));
                assert_eq!(e.refunds[0].status, ObservedRefundStatus::Succeeded);
            }
            other => panic!("expected PaymentRefunded, got {other:?}"),
        }
    }

    #[test]
    fn parse_event_maps_charged_back_to_a_chargeback_reversal() {
        let c = connector(None);
        let mock = adapter(&c);
        let payload = br#"{"id":"ev4","kind":"payment_charged_back","external_id":"pi_4","transaction_id":"tx_4"}"#;
        let ev = mock
            .parse_event(payload, &HeaderMap::new())
            .unwrap()
            .unwrap();
        match ev.kind {
            NormalizedEventKind::PaymentReversed(e) => {
                assert_eq!(e.kind, ReversalKind::Chargeback);
                assert_eq!(e.cause.as_deref(), Some("mock_chargeback"));
            }
            other => panic!("expected PaymentReversed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_refund_resolves_a_minted_id() {
        let c = connector(None);
        let id = MockRefundRef::mint(300, "EUR");
        let snapshot = adapter(&c).fetch_refund(&id, None).await.unwrap();
        assert_eq!(snapshot.amount_minor, 300);
        assert_eq!(snapshot.currency, "EUR");
        assert!(matches!(snapshot.status, RemoteRefundStatus::Succeeded));
    }

    #[tokio::test]
    async fn fetch_refund_rejects_an_id_it_never_minted() {
        let c = connector(None);
        let err = adapter(&c)
            .fetch_refund("re_from_another_provider", None)
            .await;
        assert!(matches!(
            err.unwrap_err().current_context(),
            ConnectorError::Unsupported { .. }
        ));
    }
}
