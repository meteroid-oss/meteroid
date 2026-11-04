//! Contract-test harness for [`PaymentConnector`] implementations.
//!
//! Validates the trait-level contract every connector must satisfy: idempotency
//! keys threaded, capabilities self-consistent, unsupported ops error rather
//! than panic. Provider-specific tests live in each adapter's own module.
//!
//! ```ignore
//! #[tokio::test]
//! async fn mock_satisfies_contract() {
//!     let connector = build_test_connector(ConnectorProviderEnum::Mock);
//!     let impl_ = MockConnector::new(ConnectorRef::from(&connector), MockPublicData::default());
//!     payment::contract::run_contract(&impl_, &connector).await;
//! }
//! ```

use super::connector::{PaymentCapabilities, PaymentConnector};
use super::error::ConnectorError;
use super::events::NormalizedEventSubscription;
use super::model::{
    ChargeRequest, CreateCustomerRequest, IdempotencyKey, MandateSetupRequest, RefundRequest,
};
use crate::domain::PaymentMethodTypeEnum;
use crate::domain::connectors::Connector;
use common_domain::ids::{BaseId, PaymentTransactionId};
use http::HeaderMap;

/// Run the full contract suite against a connector implementation. Panics on
/// any contract violation so the calling test fails clearly.
pub async fn run_contract(impl_: &dyn PaymentConnector, connector: &Connector) {
    assert_capabilities_consistent(impl_.capabilities());

    let test_customer = make_test_customer(connector);
    let test_connection = make_test_connection(connector);

    // ── CustomerOps ─────────────────────────────────────────────────
    let request = CreateCustomerRequest {
        idempotency_key: IdempotencyKey::new(format!("test:{}", test_customer.id.as_base62())),
    };
    let external = impl_
        .create_customer(&test_customer, request)
        .await
        .expect("create_customer must succeed on a happy-path test");
    assert!(
        !external.external_id.is_empty(),
        "external_id must not be empty"
    );

    // ── MandateOps ──────────────────────────────────────────────────
    if impl_.capabilities().supports_mandates {
        let setup = impl_
            .initiate_mandate_setup(
                &test_connection,
                MandateSetupRequest {
                    descriptor: None,
                    webhook_url: None,
                    purpose: crate::adapters::payment::model::SetupPurpose::Settings,
                    payment_methods: &[PaymentMethodTypeEnum::Card],
                    idempotency_key: IdempotencyKey::new(format!(
                        "test_mandate:{}",
                        test_connection.id.as_base62()
                    )),
                    return_url: None,
                    invoice_id: None,
                    checkout: None,
                    invoice_payment: None,
                    currency: Some("USD".to_string()),
                },
            )
            .await
            .expect("initiate_mandate_setup must succeed when supports_mandates");
        match &setup {
            super::model::MandateSetupInstruction::EmbeddedClientSecret { intent_id, .. }
            | super::model::MandateSetupInstruction::HostedRedirect { intent_id, .. }
            | super::model::MandateSetupInstruction::EmbeddedDropIn { intent_id, .. } => {
                assert!(!intent_id.is_empty(), "intent_id must be present");
            }
        }
    }

    // ── PaymentOps ──────────────────────────────────────────────────
    let charge = impl_
        .charge_off_session(ChargeRequest {
            descriptor: None,
            webhook_url: None,
            transaction_id: PaymentTransactionId::new(),
            customer_external_id: &external.external_id,
            payment_method_external_id: "pm_test",
            payment_method_type: PaymentMethodTypeEnum::Card,
            amount_minor: 1234,
            currency: "USD",
            idempotency_key: IdempotencyKey::new("test_charge_1"),
            on_session: false,
        })
        .await
        .expect("charge_off_session must return an outcome, not error, on a happy-path test");
    // Any outcome variant is contract-conforming, including a failure.
    let _ = charge;

    // ── WebhookOps ──────────────────────────────────────────────────
    // Empty headers + payload must surface a typed error, not panic.
    let result = impl_.verify_signature(
        b"",
        &HeaderMap::new(),
        &secrecy::SecretString::from("any-secret".to_string()),
    );
    if let Err(report) = result {
        // Acceptable kinds for an empty payload; anything else is a contract bug.
        assert!(
            matches!(
                report.current_context(),
                ConnectorError::SignatureMissing
                    | ConnectorError::SignatureVerification
                    | ConnectorError::PayloadDecode(_)
            ) || matches!(report.current_context(), ConnectorError::Unsupported { .. }),
            "verify_signature(empty) returned an unexpected error kind: {report:?}"
        );
    }
    // Implementations that accept the empty payload (mock) are also fine.

    // ── Capability honesty: refunds ─────────────────────────────────
    // Only the negative case is exercised: issuing a real refund needs a settled
    // charge the harness has no way to produce.
    if !impl_.capabilities().supports_refunds() {
        let result = impl_
            .refund(RefundRequest {
                external_transaction_id: "tx_contract_test",
                amount_minor: 1234,
                currency: "USD",
                reason: None,
                idempotency_key: IdempotencyKey::new("test_refund_1"),
                metadata: Vec::new(),
            })
            .await;
        assert!(
            matches!(
                result.as_ref().err().map(|r| r.current_context()),
                Some(ConnectorError::Unsupported { .. })
            ),
            "connector advertises !supports_refunds but refund() didn't return Unsupported"
        );
    }

    // ── Capability honesty: self-registration ───────────────────────
    if !impl_.capabilities().supports_self_webhook_registration() {
        let result = impl_
            .register_webhook(
                "https://example.invalid/hook",
                &[NormalizedEventSubscription::Payments],
            )
            .await;
        assert!(
            matches!(
                result.as_ref().err().map(|r| r.current_context()),
                Some(ConnectorError::Unsupported { .. })
            ),
            "connector advertises !supports_self_webhook_registration but register_webhook didn't return Unsupported"
        );
    }
}

/// The capability combinations the type system can't rule out. Most of what this used to check
/// — partial refunds implying refunds, a webhook secret implying webhooks, a non-zero replay
/// window, hosted completion implying a hosted redirect — is now unrepresentable, so what is
/// left is the two facts that still depend on an author filling the table in honestly.
pub fn assert_capabilities_consistent(caps: &PaymentCapabilities) {
    assert!(
        caps.refund_replay_window.is_none() || caps.supports_refunds(),
        "a provider that cannot refund cannot claim a refund replay window"
    );
    assert!(
        !caps.supported_payment_methods.is_empty(),
        "a connector with no supported_payment_methods can't accept money — likely a config bug"
    );
    if caps.supports_3ds {
        assert!(
            caps.supports_cards(),
            "supports_3ds implies cards are among supported_payment_methods"
        );
    }
}

// ── test fixtures ──────────────────────────────────────────────────

fn make_test_customer(connector: &Connector) -> crate::domain::Customer {
    use common_domain::ids::{CustomerId, InvoicingEntityId};
    crate::domain::Customer {
        id: CustomerId::new(),
        tenant_id: connector.tenant_id,
        name: "Contract Test Customer".to_string(),
        customer_type: crate::domain::enums::CustomerType::Company,
        first_name: None,
        last_name: None,
        payment_reminders_opt_out: false,
        payment_reminder_sequence_id: None,
        alias: None,
        billing_email: Some("ct@example.invalid".into()),
        invoicing_emails: vec![],
        phone: None,
        currency: "USD".into(),
        preferred_locales: Vec::new(),
        billing_address: None,
        shipping_address: None,
        created_at: chrono::Utc::now().naive_utc(),
        updated_at: None,
        archived_at: None,
        invoicing_entity_id: InvoicingEntityId::new(),
        vat_number: None,
        legal_number: None,
        buyer_reference: None,
        current_payment_method_id: None,
        tax_status: crate::domain::CustomerTaxStatus::Taxable,
        exemption_reason: None,
        custom_taxes: vec![],
        custom_properties: serde_json::Value::Null,
        vat_number_format_valid: false,
        vat_number_validation_status: None,
        vat_number_checked_at: None,
        vat_number_vies_check: None,
        connected_account_id: None,
        conn_meta: None,
        owner_user_id: None,
    }
}

fn make_test_connection(connector: &Connector) -> crate::domain::CustomerConnection {
    use common_domain::ids::{CustomerConnectionId, CustomerId};
    crate::domain::CustomerConnection {
        id: CustomerConnectionId::new(),
        customer_id: CustomerId::new(),
        connector_id: connector.id,
        external_customer_id: "cus_test_external".into(),
        supported_payment_types: Some(vec![PaymentMethodTypeEnum::Card]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::payment::connector::ConnectorRef;
    use crate::adapters::payment::connector::{
        ConnectorIdentity, HostedSetupCompletion, MandateSetupMode, WebhookOps,
    };
    use crate::adapters::payment::factory::provider_capabilities;
    use crate::adapters::payment::{
        GoCardlessConnector, MockConnector, MollieConnector, StancerConnector,
    };
    use crate::domain::connectors::{
        Connector, CredentialState, GocardlessEnvironment, GocardlessPublicData,
        GocardlessSensitiveData, MockPublicData, MollieSensitiveData, StancerSensitiveData,
    };
    use crate::domain::connectors::{
        ConnectorConfig, MockSensitiveData, PaymentConnectorConfig, PaymentProviderConfig,
    };
    use crate::domain::enums::PaymentProvider;

    use chrono::NaiveDateTime;
    use common_domain::ids::{ConnectorId, TenantId};

    #[tokio::test]
    async fn mock_satisfies_contract() {
        let connector = Connector {
            status: crate::domain::enums::ConnectorStatusEnum::Active,
            credentials: CredentialState::Unverified,
            webhook_route: None,
            id: ConnectorId::new(),
            created_at: NaiveDateTime::default(),
            tenant_id: TenantId::new(),
            alias: "contract-mock".into(),
            config: ConnectorConfig::Payment(PaymentConnectorConfig::configured(
                PaymentProviderConfig::Mock(MockPublicData::default(), MockSensitiveData {}),
            )),
        };
        let impl_ = MockConnector::new(ConnectorRef::from(&connector), MockPublicData::default());
        run_contract(&impl_, &connector).await;
    }

    /// Confirms the GoCardless cap matrix is internally consistent. Runtime ops
    /// aren't exercised here since they'd hit the unreachable sandbox API.
    #[test]
    fn gocardless_capabilities_consistent() {
        let caps = provider_capabilities(PaymentProvider::Gocardless);
        assert_capabilities_consistent(caps);
        assert!(!caps.supports_cards(), "GC has no card support");
        assert!(caps.supports_mandates);
        assert!(
            caps.asynchronous_settlement,
            "GC settlement is multi-day async"
        );
        assert!(
            !caps.supports_self_webhook_registration(),
            "GC webhook endpoints are dashboard-managed"
        );
        assert_eq!(caps.mandate_setup_mode(), MandateSetupMode::HostedRedirect);
    }

    /// Stripe refunds are implemented end to end: `POST /v1/refunds` takes an
    /// explicit amount, and `GET /v1/refunds/{id}` backs the reconcile sweep.
    #[test]
    fn stripe_capabilities_consistent() {
        let caps = provider_capabilities(PaymentProvider::Stripe);
        assert_capabilities_consistent(caps);
        assert!(caps.supports_cards(), "Stripe is a card provider");
        assert!(caps.supports_mandates);
        assert!(caps.supports_refunds(), "refund() is implemented");
        assert!(
            caps.supports_partial_refunds(),
            "Stripe refunds take an explicit amount"
        );
        assert!(
            caps.supports_self_webhook_registration(),
            "Stripe endpoints are self-registered on connect"
        );
        assert_eq!(
            caps.mandate_setup_mode(),
            MandateSetupMode::EmbeddedClientSecret
        );
    }

    /// Confirms the Stancer cap matrix is internally consistent. Runtime ops
    /// aren't exercised here since they'd hit the real Stancer API.
    #[test]
    fn stancer_capabilities_consistent() {
        let caps = provider_capabilities(PaymentProvider::Stancer);
        assert_capabilities_consistent(caps);
        assert!(caps.supports_cards(), "Stancer is a card provider");
        assert!(caps.supports_mandates);
        assert!(
            caps.asynchronous_settlement,
            "Stancer capture resolves asynchronously with no push channel"
        );
        assert!(
            !caps.supports_self_webhook_registration(),
            "Stancer has no webhook mechanism at all"
        );
        assert!(
            caps.supports_partial_refunds(),
            "POST /v2/refunds/ takes any amount"
        );
        assert_eq!(caps.mandate_setup_mode(), MandateSetupMode::HostedRedirect);
    }

    /// The pending-intent sweeper keys off this capability: only
    /// `PollingRequired` providers persist the intent id and get swept;
    /// `WebhookBacked` providers must never be swept.
    #[test]
    fn hosted_setup_completion_matches_webhook_reality() {
        assert_eq!(
            provider_capabilities(PaymentProvider::Stancer).hosted_setup_completion(),
            Some(HostedSetupCompletion::PollingRequired),
            "Stancer has no webhooks; polling is its only completion backstop"
        );
        assert_eq!(
            provider_capabilities(PaymentProvider::Gocardless).hosted_setup_completion(),
            Some(HostedSetupCompletion::WebhookBacked),
            "billing_requests.fulfilled backs GoCardless hosted setups"
        );
        // Stripe has no hosted redirect at all: its setup finishes client-side, so there is no
        // hosted completion to describe. What the sweeper cares about is that it is never swept.
        assert_eq!(
            provider_capabilities(PaymentProvider::Stripe).hosted_setup_completion(),
            None
        );
        assert!(!provider_capabilities(PaymentProvider::Stripe).completes_pending_hosted_intents());
    }

    /// Hosted invoice collection is only safe if completion records the capture; otherwise the
    /// invoice would also be charged off-session.
    #[test]
    fn hosted_invoice_payment_matches_completion_design() {
        let stancer = provider_capabilities(PaymentProvider::Stancer);
        let mollie = provider_capabilities(PaymentProvider::Mollie);
        let gocardless = provider_capabilities(PaymentProvider::Gocardless);
        let stripe = provider_capabilities(PaymentProvider::Stripe);

        assert!(stancer.supports_hosted_invoice_payment());
        assert!(
            mollie.supports_hosted_invoice_payment(),
            "Mollie stamps the invoice transaction into the first payment"
        );
        assert!(
            !gocardless.supports_hosted_invoice_payment(),
            "GoCardless charges the invoice off-session once the mandate exists"
        );
        assert!(!stripe.supports_hosted_invoice_payment());

        assert!(stancer.completes_pending_hosted_intents());
        assert!(mollie.completes_pending_hosted_intents());
        assert!(!gocardless.completes_pending_hosted_intents());
        assert!(!stripe.completes_pending_hosted_intents());
    }

    /// No webhook self-registration; unsigned webhooks need the URL token.
    #[tokio::test]
    async fn mollie_capabilities_and_webhook_contract() {
        let connector = Connector {
            status: crate::domain::enums::ConnectorStatusEnum::Active,
            credentials: CredentialState::Unverified,
            webhook_route: None,
            id: ConnectorId::new(),
            created_at: NaiveDateTime::default(),
            tenant_id: TenantId::new(),
            alias: "contract-mollie".into(),
            config: ConnectorConfig::Payment(PaymentConnectorConfig::configured(
                PaymentProviderConfig::Mollie(MollieSensitiveData {
                    api_key: "test_fake".into(),
                    webhook_token: "token".into(),
                }),
            )),
        };
        let impl_ = MollieConnector::new(
            ConnectorRef::from(&connector),
            MollieSensitiveData {
                api_key: "test_fake".into(),
                webhook_token: "token".into(),
            },
        );
        let caps = impl_.capabilities();
        assert_capabilities_consistent(caps);
        assert!(caps.supports_cards(), "Mollie is a card provider");
        assert!(caps.supports_mandates);
        assert!(caps.asynchronous_settlement);
        assert!(!caps.supports_self_webhook_registration());
        assert_eq!(caps.mandate_setup_mode(), MandateSetupMode::HostedRedirect);
        assert_eq!(
            caps.hosted_setup_completion(),
            Some(HostedSetupCompletion::WebhookBacked),
            "the payment webhook backstops a lost return"
        );

        let result = impl_
            .register_webhook(
                "https://example.invalid/hook",
                &[NormalizedEventSubscription::Payments],
            )
            .await;
        assert!(matches!(
            result.as_ref().err().map(|r| r.current_context()),
            Some(ConnectorError::Unsupported { .. })
        ));

        let verify = impl_.verify_signature(
            b"id=tr_x",
            &HeaderMap::new(),
            &secrecy::SecretString::from("token".to_string()),
        );
        assert!(
            matches!(
                verify.as_ref().err().map(|r| r.current_context()),
                Some(ConnectorError::SignatureMissing)
            ),
            "an unsigned ping without the URL token must be rejected"
        );

        // Other adapters keep the default (no resource resolution).
        let stancer_connector = Connector {
            status: crate::domain::enums::ConnectorStatusEnum::Active,
            credentials: CredentialState::Unverified,
            webhook_route: None,
            config: ConnectorConfig::Payment(PaymentConnectorConfig::configured(
                PaymentProviderConfig::Stancer(StancerSensitiveData {
                    api_secret_key: "stest_fake".into(),
                }),
            )),
            ..connector
        };
        let unsupported = StancerConnector::new(
            ConnectorRef::from(&stancer_connector),
            StancerSensitiveData {
                api_secret_key: "stest_fake".into(),
            },
        )
        .resolve_resource_change("paym_x")
        .await;
        assert!(matches!(
            unsupported.as_ref().err().map(|r| r.current_context()),
            Some(ConnectorError::Unsupported { .. })
        ));
    }

    /// Stancer has no webhook mechanism: registration must return Unsupported
    /// and signature verification must reject unconditionally.
    #[tokio::test]
    async fn stancer_register_webhook_is_unsupported() {
        let connector = Connector {
            status: crate::domain::enums::ConnectorStatusEnum::Active,
            credentials: CredentialState::Unverified,
            webhook_route: None,
            id: ConnectorId::new(),
            created_at: NaiveDateTime::default(),
            tenant_id: TenantId::new(),
            alias: "contract-stancer".into(),
            config: ConnectorConfig::Payment(PaymentConnectorConfig::configured(
                PaymentProviderConfig::Stancer(StancerSensitiveData {
                    api_secret_key: "stest_fake".into(),
                }),
            )),
        };
        let impl_ = StancerConnector::new(
            ConnectorRef::from(&connector),
            StancerSensitiveData {
                api_secret_key: "stest_fake".into(),
            },
        );
        let result = impl_
            .register_webhook(
                "https://example.invalid/hook",
                &[NormalizedEventSubscription::Payments],
            )
            .await;
        assert!(
            matches!(
                result.as_ref().err().map(|r| r.current_context()),
                Some(ConnectorError::Unsupported { .. })
            ),
            "Stancer register_webhook must return Unsupported (got: {result:?})"
        );

        let verify = impl_.verify_signature(
            b"{}",
            &HeaderMap::new(),
            &secrecy::SecretString::from("no-such-secret".to_string()),
        );
        assert!(
            matches!(
                verify.as_ref().err().map(|r| r.current_context()),
                Some(ConnectorError::SignatureVerification)
            ),
            "no Stancer webhook signature can ever verify"
        );
    }

    /// With the cap off, register_webhook must return Unsupported, not synthesise
    /// data. Pinned for GoCardless, the only connector hitting this branch in prod.
    #[tokio::test]
    async fn gocardless_register_webhook_is_unsupported() {
        let connector = Connector {
            status: crate::domain::enums::ConnectorStatusEnum::Active,
            credentials: CredentialState::Unverified,
            webhook_route: None,
            id: ConnectorId::new(),
            created_at: NaiveDateTime::default(),
            tenant_id: TenantId::new(),
            alias: "contract-gc".into(),
            config: ConnectorConfig::Payment(PaymentConnectorConfig::configured(
                PaymentProviderConfig::Gocardless(
                    GocardlessPublicData {
                        creditor_id: None,
                        environment: GocardlessEnvironment::Sandbox,
                    },
                    GocardlessSensitiveData {
                        access_token: "fake".into(),
                        webhook_secret: "fake".into(),
                    },
                ),
            )),
        };
        let impl_ = GoCardlessConnector::new(
            ConnectorRef::from(&connector),
            GocardlessPublicData {
                creditor_id: None,
                environment: GocardlessEnvironment::Sandbox,
            },
            GocardlessSensitiveData {
                access_token: "fake".into(),
                webhook_secret: "fake".into(),
            },
        );
        let result = impl_
            .register_webhook(
                "https://example.invalid/hook",
                &[NormalizedEventSubscription::Payments],
            )
            .await;
        assert!(
            matches!(
                result.as_ref().err().map(|r| r.current_context()),
                Some(ConnectorError::Unsupported { .. })
            ),
            "GoCardless register_webhook must return Unsupported (got: {result:?})"
        );
    }
}
