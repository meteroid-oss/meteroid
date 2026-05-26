//! Mock connector used by tests. Behavior is driven by the [`MockPublicData`]
//! attached to the [`Connector`], so a single instance can be configured for
//! success or failure scenarios per-test.

use super::connector::{
    ConnectorCapabilities, ConnectorIdentity, CustomerOps, MandateOps, MandateSetupMode,
    PaymentOps, ReconcileOps, RefundOps, WebhookOps,
};
use super::error::ConnectorError;
use super::events::{NormalizedEventSubscription, NormalizedWebhookEvent};
use super::model::{
    ChargeFailure, ChargeOutcome, ChargeReceipt, ChargeRequest, CreateCustomerRequest,
    DeclineKind, ExternalCustomerRef, MandateSetupInstruction, MandateSetupRequest,
    PaymentMethodSnapshot, RefundOutcome, RefundReceipt, RefundRequest, RegisteredWebhook,
    RemoteTransactionStatus,
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
    supports_3ds: false,
    supports_disputes: true,
    // Mock has no real provider so it can synthesize everything, including
    // pretending to register webhook endpoints. Keeping this `true` makes the
    // capability flag honest about the behaviour returned by `register_webhook`.
    supports_self_webhook_registration: true,
    asynchronous_settlement: false,
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

        if self.config.fail_payment_intent {
            return Ok(ChargeOutcome::Failed(ChargeFailure {
                external_id: Some(external_id),
                code: Some("mock_failure".to_string()),
                message: "Mock payment failure (configured)".to_string(),
                retryable: false,
                decline_kind: DeclineKind::Other,
                provider_request_id: None,
            }));
        }

        Ok(ChargeOutcome::Succeeded(ChargeReceipt {
            external_id,
            amount_received_minor: request.amount_minor,
            processed_at: chrono::Utc::now().naive_utc(),
            provider_request_id: None,
        }))
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

    fn parse_event(
        &self,
        _connector: &Connector,
        _payload: &[u8],
        _headers: &HeaderMap,
    ) -> Result<Option<NormalizedWebhookEvent>, Report<ConnectorError>> {
        Ok(None)
    }
}
