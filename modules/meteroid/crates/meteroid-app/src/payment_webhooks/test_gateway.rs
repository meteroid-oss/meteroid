//! Backoffice-driven provider events for the test gateway. A real provider would push these as
//! webhooks; here the merchant clicks, the mock adapter parses its own envelope, and the event
//! takes the production dispatch path, so nothing is settled through a side door.

use crate::payment_webhooks::event_handler::handle_normalized_event;
use common_domain::ids::BaseId;
use error_stack::{Report, ResultExt, bail};
use http::HeaderMap;
use meteroid_store::adapters::payment::initialize_payment_connector;
use meteroid_store::domain::connectors::Connector;
use meteroid_store::domain::enums::{PaymentProvider, PaymentStatusEnum};
use meteroid_store::domain::payment_transactions::PaymentTransaction;
use meteroid_store::{Services, Store};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestGatewayEvent {
    Succeeded,
    Failed,
    Refunded,
    /// Full claw-back (GoCardless `charged_back` shape): lost to the payer's claim, the invoice
    /// stays paid.
    ChargedBack,
}

#[derive(Debug, thiserror::Error)]
pub enum TestGatewayError {
    #[error("only test gateway transactions can receive simulated events")]
    NotTestGateway,
    #[error("a {0:?} transaction cannot receive a {1:?} event")]
    InvalidState(PaymentStatusEnum, TestGatewayEvent),
    #[error("the simulated event could not be dispatched")]
    Dispatch,
}

impl TestGatewayEvent {
    fn accepts(self, status: &PaymentStatusEnum) -> bool {
        match self {
            Self::Succeeded | Self::Failed => *status == PaymentStatusEnum::Pending,
            Self::Refunded | Self::ChargedBack => *status == PaymentStatusEnum::Settled,
        }
    }

    /// The envelope the mock adapter parses (`MockWebhookEvent`). `Refunded` reports the new
    /// cumulative refunded total — a provider-side refund of whatever still isn't refunded or
    /// reversed. `ChargedBack` is always the full amount.
    fn envelope(self, tx: &PaymentTransaction) -> serde_json::Value {
        match self {
            Self::Succeeded | Self::Failed => serde_json::json!({
                "id": format!("mock_sim_{}", Uuid::now_v7()),
                "kind": if self == Self::Succeeded { "payment_succeeded" } else { "payment_failed" },
                "transaction_id": tx.id.as_base62(),
                "external_id": tx.provider_transaction_id,
                "amount": tx.amount,
                "currency": tx.currency,
            }),
            Self::Refunded => {
                let delta = tx.amount - tx.amount_refunded - tx.amount_reversed;
                let cumulative = tx.amount_refunded + delta;
                serde_json::json!({
                    "id": format!("mock_sim_{}", Uuid::now_v7()),
                    "kind": "payment_refunded",
                    "transaction_id": tx.id.as_base62(),
                    "external_id": tx.provider_transaction_id,
                    "amount": cumulative,
                    "currency": tx.currency,
                    "refunds": [{
                        "id": format!("mock_re_{}", Uuid::now_v7()),
                        "amount": delta,
                        "succeeded": true,
                    }],
                })
            }
            Self::ChargedBack => serde_json::json!({
                "id": format!("mock_sim_{}", Uuid::now_v7()),
                "kind": "payment_charged_back",
                "transaction_id": tx.id.as_base62(),
                "external_id": tx.provider_transaction_id,
                "currency": tx.currency,
            }),
        }
    }
}

pub async fn simulate(
    store: &Store,
    services: &Services,
    connector: &Connector,
    tx: &PaymentTransaction,
    event: TestGatewayEvent,
) -> Result<(), Report<TestGatewayError>> {
    if connector.payment_provider() != Some(PaymentProvider::Mock) {
        bail!(TestGatewayError::NotTestGateway);
    }
    if !event.accepts(&tx.status) {
        bail!(TestGatewayError::InvalidState(tx.status.clone(), event));
    }

    let connector_impl =
        initialize_payment_connector(connector).change_context(TestGatewayError::Dispatch)?;
    let payload =
        serde_json::to_vec(&event.envelope(tx)).change_context(TestGatewayError::Dispatch)?;
    let normalized = connector_impl
        .parse_event(&payload, &HeaderMap::new())
        .change_context(TestGatewayError::Dispatch)?
        .ok_or_else(|| Report::new(TestGatewayError::Dispatch))?;

    handle_normalized_event(
        normalized,
        connector,
        connector_impl.as_ref(),
        store.clone(),
        services,
    )
    .await
    .change_context(TestGatewayError::Dispatch)
}
