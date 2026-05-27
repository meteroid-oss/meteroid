//! Mock connector used by tests. Behavior is driven by the [`MockPublicData`]
//! attached to the [`Connector`], so a single instance can be configured for
//! success or failure scenarios per-test.

use super::connector::{
    ConnectorCapabilities, ConnectorIdentity, CustomerOps, MandateOps, MandateSetupMode,
    PaymentOps, ReconcileOps, RefundOps, WebhookOps,
};
use super::error::ConnectorError;
use super::events::{
    NormalizedEventKind, NormalizedEventSubscription, NormalizedWebhookEvent,
    PaymentFailedEvent, PaymentMethodAttachedEvent, PaymentMethodUpdatedEvent, PaymentPendingEvent,
    PaymentRequiresActionEvent, PaymentSucceededEvent,
};
use super::model::{
    ChargeAcknowledged, ChargeFailure, ChargeOutcome, ChargeReceipt, ChargeRequest,
    CreateCustomerRequest, DeclineKind, ExternalCustomerRef, MandateSetupInstruction,
    MandateSetupRequest, PaymentMethodSnapshot, RefundOutcome, RefundReceipt, RefundRequest,
    RegisteredWebhook, RemoteTransactionStatus, RequiresActionInstruction,
};
use crate::domain::connectors::{Connector, MockPublicData, ProviderData};
use crate::domain::enums::ConnectorProviderEnum;
use crate::domain::{Customer, CustomerConnection, PaymentMethodTypeEnum};
use async_trait::async_trait;
use common_domain::ids::BaseId;
use error_stack::Report;
use http::HeaderMap;
use secrecy::SecretString;
use uuid::Uuid;

const MOCK_CAPABILITIES: ConnectorCapabilities = ConnectorCapabilities {
    supports_cards: true,
    supports_mandates: true,
    supports_refunds: true,
    supports_partial_refunds: true,
    supports_3ds: true,
    supports_disputes: true,
    // Mock has no real provider so it can synthesize everything, including
    // pretending to register webhook endpoints. Keeping this `true` makes the
    // capability flag honest about the behaviour returned by `register_webhook`.
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

#[derive(Debug, Clone, Default)]
pub struct MockConnector {
    config: MockPublicData,
}

impl MockConnector {
    pub fn from_config(config: MockPublicData) -> Self {
        Self { config }
    }

    pub fn from_connector(connector: &Connector) -> Self {
        let config = match &connector.data {
            Some(ProviderData::Mock(data)) => data.clone(),
            _ => MockPublicData::default(),
        };
        Self { config }
    }
}

impl ConnectorIdentity for MockConnector {
    fn provider(&self) -> ConnectorProviderEnum {
        ConnectorProviderEnum::Mock
    }

    fn capabilities(&self) -> &ConnectorCapabilities {
        &MOCK_CAPABILITIES
    }
}

#[async_trait]
impl CustomerOps for MockConnector {
    async fn create_customer(
        &self,
        _connector: &Connector,
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
        _connector: &Connector,
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
            publishable_key: SecretString::from("mock_pk_test_key".to_string()),
        })
    }

    async fn fetch_payment_method(
        &self,
        _connector: &Connector,
        external_payment_method_id: &str,
        _external_customer_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        Ok(PaymentMethodSnapshot {
            external_payment_method_id: external_payment_method_id.to_string(),
            payment_method_type: PaymentMethodTypeEnum::Card,
            account_number_hint: None,
            card_brand: Some("mock_visa".to_string()),
            card_last4: Some("4242".to_string()),
            card_exp_month: Some(12),
            card_exp_year: Some(2030),
            meteroid_connection_id: None,
            meteroid_customer_id: None,
        })
    }

    async fn complete_mandate_setup(
        &self,
        _connector: &Connector,
        intent_id: &str,
    ) -> Result<PaymentMethodSnapshot, Report<ConnectorError>> {
        // Mock supports both embedded and hosted flows — pretend a redirect
        // returned and synthesise a snapshot.
        Ok(PaymentMethodSnapshot {
            external_payment_method_id: format!("mock_md_{intent_id}"),
            payment_method_type: PaymentMethodTypeEnum::DirectDebitSepa,
            account_number_hint: Some("0009".to_string()),
            card_brand: None,
            card_last4: None,
            card_exp_month: None,
            card_exp_year: None,
            meteroid_connection_id: None,
            meteroid_customer_id: None,
        })
    }
}

#[async_trait]
impl PaymentOps for MockConnector {
    async fn charge_off_session(
        &self,
        _connector: &Connector,
        request: ChargeRequest<'_>,
    ) -> Result<ChargeOutcome, Report<ConnectorError>> {
        let external_id = format!("mock_pi_{}", Uuid::now_v7());

        let behavior = if self.config.fail_payment_intent {
            "failed"
        } else {
            self.config.charge_behavior.as_deref().unwrap_or("succeeded")
        };

        let outcome = match behavior {
            "failed" => ChargeOutcome::Failed(ChargeFailure {
                external_id: Some(external_id),
                code: Some("mock_failure".to_string()),
                message: "Mock payment failure (configured)".to_string(),
                retryable: false,
                decline_kind: DeclineKind::Other,
                provider_request_id: None,
            }),
            "pending" => ChargeOutcome::Pending(ChargeAcknowledged {
                external_id,
                provider_request_id: None,
            }),
            "requires_action" => {
                ChargeOutcome::RequiresAction(RequiresActionInstruction::ClientSecret {
                    external_id,
                    client_secret: format!("mock_secret_{}", Uuid::now_v7()),
                    publishable_key: SecretString::from("mock_pk_test_key".to_string()),
                })
            }
            _ => ChargeOutcome::Succeeded(ChargeReceipt {
                external_id,
                amount_received_minor: request.amount_minor,
                processed_at: chrono::Utc::now().naive_utc(),
                provider_request_id: None,
            }),
        };
        Ok(outcome)
    }
}

#[async_trait]
impl RefundOps for MockConnector {
    async fn refund(
        &self,
        _connector: &Connector,
        request: RefundRequest<'_>,
    ) -> Result<RefundOutcome, Report<ConnectorError>> {
        Ok(RefundOutcome::Succeeded(RefundReceipt {
            external_refund_id: format!("mock_re_{}", Uuid::now_v7()),
            amount_refunded_minor: request.amount_minor,
            processed_at: chrono::Utc::now().naive_utc(),
            provider_request_id: None,
        }))
    }
}

#[async_trait]
impl ReconcileOps for MockConnector {
    async fn fetch_transaction_status(
        &self,
        _connector: &Connector,
        _external_transaction_id: &str,
    ) -> Result<RemoteTransactionStatus, Report<ConnectorError>> {
        Ok(RemoteTransactionStatus::Pending)
    }
}

#[async_trait]
impl WebhookOps for MockConnector {
    async fn register_webhook(
        &self,
        _connector: &Connector,
        _url: &str,
        _subscriptions: &[NormalizedEventSubscription],
    ) -> Result<RegisteredWebhook, Report<ConnectorError>> {
        Ok(RegisteredWebhook {
            endpoint_id: format!("mock_we_{}", Uuid::now_v7()),
            secret: SecretString::from(format!("mock_whsec_{}", Uuid::now_v7())),
        })
    }

    async fn unregister_webhook(
        &self,
        _connector: &Connector,
        _endpoint_id: &str,
    ) -> Result<(), Report<ConnectorError>> {
        Ok(())
    }

    async fn sync_webhook_events(
        &self,
        _connector: &Connector,
        _endpoint_id: &str,
        _subscriptions: &[NormalizedEventSubscription],
    ) -> Result<(), Report<ConnectorError>> {
        Ok(())
    }

    fn verify_signature(
        &self,
        _connector: &Connector,
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
        _connector: &Connector,
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
                meteroid_transaction_id: e.transaction_id.clone(),
            }),
            "payment_pending" => NormalizedEventKind::PaymentPending(PaymentPendingEvent {
                external_transaction_id: e.external_id.clone().unwrap_or_default(),
                meteroid_transaction_id: e.transaction_id.clone(),
            }),
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
            occurred_at: chrono::Utc::now(),
            kind,
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
    #[serde(default)]
    amount: Option<i64>,
    #[serde(default)]
    currency: Option<String>,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::{ConnectorId, TenantId};

    fn connector(behavior: Option<&str>) -> Connector {
        Connector {
            id: ConnectorId::new(),
            created_at: chrono::NaiveDateTime::default(),
            tenant_id: TenantId::new(),
            alias: "mock".into(),
            connector_type: crate::domain::enums::ConnectorTypeEnum::PaymentProvider,
            provider: ConnectorProviderEnum::Mock,
            data: Some(ProviderData::Mock(MockPublicData {
                charge_behavior: behavior.map(str::to_string),
                ..Default::default()
            })),
            sensitive: None,
        }
    }

    async fn charge(behavior: Option<&str>) -> ChargeOutcome {
        let c = connector(behavior);
        MockConnector::from_connector(&c)
            .charge_off_session(
                &c,
                ChargeRequest {
                    transaction_id: common_domain::ids::PaymentTransactionId::new(),
                    customer_external_id: "cus",
                    payment_method_external_id: "pm",
                    payment_method_type: PaymentMethodTypeEnum::Card,
                    amount_minor: 500,
                    currency: "EUR",
                    idempotency_key: super::super::model::IdempotencyKey::new("k"),
                    on_session: true,
                },
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn charge_behaviors() {
        assert!(matches!(charge(None).await, ChargeOutcome::Succeeded(_)));
        assert!(matches!(charge(Some("pending")).await, ChargeOutcome::Pending(_)));
        assert!(matches!(charge(Some("failed")).await, ChargeOutcome::Failed(_)));
        assert!(matches!(
            charge(Some("requires_action")).await,
            ChargeOutcome::RequiresAction(_)
        ));
    }

    #[test]
    fn parse_event_maps_kinds() {
        let c = connector(None);
        let mock = MockConnector::from_connector(&c);
        let payload = br#"{"id":"ev1","kind":"payment_succeeded","external_id":"pi_1","transaction_id":"tx_1","amount":500,"currency":"EUR"}"#;
        let ev = mock
            .parse_event(&c, payload, &HeaderMap::new())
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
        let ev = mock.parse_event(&c, ra, &HeaderMap::new()).unwrap().unwrap();
        assert!(matches!(
            ev.kind,
            NormalizedEventKind::PaymentRequiresAction(_)
        ));
    }
}
