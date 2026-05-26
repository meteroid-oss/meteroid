//! Contract-test harness for [`PaymentConnector`] implementations.
//!
//! Every connector — Stripe today, GoCardless and Adyen next — must satisfy
//! the same observable behavior at the trait level. Provider-specific tests
//! (real HTTP, payload shapes, signature schemes) live in each adapter's own
//! test module; this harness validates the *contract* the trait promises to
//! the core: idempotency keys are threaded, capabilities are self-consistent,
//! unsupported operations return errors instead of panicking, etc.
//!
//! Usage:
//! ```ignore
//! #[tokio::test]
//! async fn mock_satisfies_contract() {
//!     let connector = build_test_connector(ConnectorProviderEnum::Mock);
//!     let impl_ = MockConnector::from_connector(&connector);
//!     payment::contract::run_contract(&impl_, &connector).await;
//! }
//! ```

use super::connector::{ConnectorCapabilities, PaymentConnector};
use super::error::ConnectorError;
use super::events::NormalizedEventSubscription;
use super::model::{ChargeRequest, CreateCustomerRequest, IdempotencyKey, MandateSetupRequest};
use crate::domain::PaymentMethodTypeEnum;
use crate::domain::connectors::Connector;
use common_domain::ids::{BaseId, PaymentTransactionId};
use error_stack::Report;
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
        .create_customer(connector, &test_customer, request)
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
                connector,
                &test_connection,
                MandateSetupRequest {
                    payment_methods: &[PaymentMethodTypeEnum::Card],
                    idempotency_key: IdempotencyKey::new(format!(
                        "test_mandate:{}",
                        test_connection.id.as_base62()
                    )),
                    return_url: None,
                },
            )
            .await
            .expect("initiate_mandate_setup must succeed when supports_mandates");
        // Each variant carries an intent_id we can correlate later.
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
        .charge_off_session(
            connector,
            ChargeRequest {
                transaction_id: PaymentTransactionId::new(),
                customer_external_id: &external.external_id,
                payment_method_external_id: "pm_test",
                payment_method_type: PaymentMethodTypeEnum::Card,
                amount_minor: 1234,
                currency: "USD",
                idempotency_key: IdempotencyKey::new("test_charge_1"),
            },
        )
        .await
        .expect("charge_off_session must return an outcome, not error, on a happy-path test");
    // The outcome variant tells us what happened — every variant is fine here
    // (a failure outcome is still a contract-conforming response).
    let _ = charge;

    // ── WebhookOps ──────────────────────────────────────────────────
    // verify_signature with empty headers + empty payload should NOT panic —
    // it must surface a typed error.
    let result = impl_.verify_signature(
        connector,
        b"",
        &HeaderMap::new(),
        &secrecy::SecretString::from("any-secret".to_string()),
    );
    if let Err(report) = result {
        // Acceptable error kinds for an empty payload; anything else is a contract bug.
        assert!(
            matches!(
                report.current_context(),
                ConnectorError::SignatureMissing
                    | ConnectorError::SignatureVerification
                    | ConnectorError::PayloadDecode(_)
            ) || matches!(
                report.current_context(),
                ConnectorError::Unsupported { .. }
            ),
            "verify_signature(empty) returned an unexpected error kind: {report:?}"
        );
    }
    // Some implementations (mock) accept the empty payload; that's also fine.

    // ── Capability honesty: self-registration ───────────────────────
    if !impl_.capabilities().supports_self_webhook_registration {
        let result = impl_
            .register_webhook(
                connector,
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

/// Capability bits should be self-consistent: e.g. partial refunds imply
/// refunds; 3DS implies cards.
pub fn assert_capabilities_consistent(caps: &ConnectorCapabilities) {
    if caps.supports_partial_refunds {
        assert!(
            caps.supports_refunds,
            "supports_partial_refunds implies supports_refunds"
        );
    }
    if caps.supports_3ds {
        assert!(caps.supports_cards, "supports_3ds implies supports_cards");
    }
    assert!(
        caps.webhook_replay_tolerance_secs > 0,
        "webhook_replay_tolerance_secs must be > 0 (0 would reject every webhook)"
    );
    assert!(
        !caps.supported_payment_methods.is_empty(),
        "a connector with no supported_payment_methods can't accept money — likely a config bug"
    );
}

// ── test fixtures ──────────────────────────────────────────────────

fn make_test_customer(connector: &Connector) -> crate::domain::Customer {
    use common_domain::ids::{CustomerId, InvoicingEntityId};
    crate::domain::Customer {
        id: CustomerId::new(),
        tenant_id: connector.tenant_id,
        name: "Contract Test Customer".to_string(),
        alias: None,
        billing_email: Some("ct@example.invalid".into()),
        invoicing_emails: vec![],
        phone: None,
        balance_value_cents: 0,
        currency: "USD".into(),
        billing_address: None,
        shipping_address: None,
        created_at: chrono::Utc::now().naive_utc(),
        created_by: uuid::Uuid::nil(),
        updated_at: None,
        updated_by: None,
        archived_at: None,
        archived_by: None,
        invoicing_entity_id: InvoicingEntityId::new(),
        vat_number: None,
        current_payment_method_id: None,
        is_tax_exempt: false,
        custom_taxes: vec![],
        vat_number_format_valid: false,
        connected_account_id: None,
        conn_meta: None,
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
    use crate::adapters::payment::connector::{
        ConnectorIdentity, MandateSetupMode, WebhookOps,
    };
    use crate::adapters::payment::{GoCardlessConnector, MockConnector};
    use crate::domain::connectors::{
        Connector, GocardlessPublicData, GocardlessSensitiveData, MockPublicData, ProviderData,
        ProviderSensitiveData,
    };
    use crate::domain::enums::{ConnectorProviderEnum, ConnectorTypeEnum};
    use chrono::NaiveDateTime;
    use common_domain::ids::{ConnectorId, TenantId};

    /// MockConnector must satisfy the trait contract.
    #[tokio::test]
    async fn mock_satisfies_contract() {
        let connector = Connector {
            id: ConnectorId::new(),
            created_at: NaiveDateTime::default(),
            tenant_id: TenantId::new(),
            alias: "contract-mock".into(),
            connector_type: ConnectorTypeEnum::PaymentProvider,
            provider: ConnectorProviderEnum::Mock,
            data: Some(ProviderData::Mock(MockPublicData::default())),
            sensitive: None,
        };
        let impl_ = MockConnector::from_connector(&connector);
        run_contract(&impl_, &connector).await;
    }

    /// GoCardless capability assertions — confirms the cap matrix is
    /// internally consistent (no cards but mandates yes, async settlement,
    /// hosted redirect mode, !self_webhook_registration).
    ///
    /// We don't exercise the runtime ops here because they'd hit
    /// `api-sandbox.gocardless.com` (which our test env can't reach);
    /// Phase 2 wires a wiremock-backed run.
    #[test]
    fn gocardless_capabilities_consistent() {
        let connector = GoCardlessConnector::new();
        let caps = connector.capabilities();
        assert_capabilities_consistent(caps);
        assert!(!caps.supports_cards, "GC has no card support");
        assert!(caps.supports_mandates);
        assert!(caps.asynchronous_settlement, "GC settlement is multi-day async");
        assert!(
            !caps.supports_self_webhook_registration,
            "GC webhook endpoints are dashboard-managed"
        );
        assert_eq!(caps.mandate_setup_mode, MandateSetupMode::HostedRedirect);
    }

    /// Honesty check: with the cap turned off, register_webhook must return
    /// Unsupported (not Ok with synthesised data). Contract harness asserts
    /// this for any connector but we pin GoCardless explicitly because it's
    /// the only one that exercises this branch in production.
    #[tokio::test]
    async fn gocardless_register_webhook_is_unsupported() {
        let connector = Connector {
            id: ConnectorId::new(),
            created_at: NaiveDateTime::default(),
            tenant_id: TenantId::new(),
            alias: "contract-gc".into(),
            connector_type: ConnectorTypeEnum::PaymentProvider,
            provider: ConnectorProviderEnum::Gocardless,
            data: Some(ProviderData::Gocardless(GocardlessPublicData {
                creditor_id: None,
                environment: "sandbox".into(),
            })),
            sensitive: Some(ProviderSensitiveData::Gocardless(GocardlessSensitiveData {
                access_token: "fake".into(),
                webhook_secret: "fake".into(),
            })),
        };
        let impl_ = GoCardlessConnector::new();
        let result = impl_
            .register_webhook(
                &connector,
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
