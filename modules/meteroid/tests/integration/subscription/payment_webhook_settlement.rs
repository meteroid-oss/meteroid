//! End-to-end, mock-driven settlement tests against a real Postgres test DB.
//!
//! These cover the *asynchronous* payment paths the synchronous-success tests
//! in `payment_methods_config.rs` don't: a charge that comes back `Pending`
//! (SEPA/ACH-style) or `requires_action` (3DS/SCA) settles only when the
//! provider's webhook arrives. We drive the real production dispatcher
//! ([`handle_normalized_event`]) with events produced by the `MockConnector`'s
//! own `parse_event`, so the test exercises parse → consolidate → outbox →
//! settlement orchestration → invoice paid, exactly as the HTTP webhook route
//! does (minus signature verification, which is unit-tested per adapter).

use http::HeaderMap;
use rstest::rstest;

use chrono::NaiveDate;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, TestEnv, subscription, test_env};

use meteroid::api_rest::webhooks::event_handler::handle_normalized_event;
use meteroid_store::adapters::payment::initialize_payment_connector;
use meteroid_store::domain::PaymentStatusEnum;
use meteroid_store::domain::connectors::{Connector, MockPublicData, ProviderData};
use meteroid_store::domain::enums::{ConnectorProviderEnum, ConnectorTypeEnum};
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;

use common_domain::ids::{BaseId, InvoiceId, PaymentTransactionId};

/// The mock Connector domain object, matching what `seed_mock_payment_provider`
/// writes. The dispatcher only reads `tenant_id`/`provider` from it; the
/// `MockConnector` built from it ignores the data for the succeeded/failed path.
fn mock_connector() -> Connector {
    Connector {
        id: MOCK_CONNECTOR_ID,
        created_at: chrono::Utc::now().naive_utc(),
        tenant_id: TENANT_ID,
        alias: "mock-payment-provider".to_string(),
        connector_type: ConnectorTypeEnum::PaymentProvider,
        provider: ConnectorProviderEnum::Mock,
        data: Some(ProviderData::Mock(MockPublicData::default())),
        sensitive: None,
    }
}

/// Parse a mock webhook envelope and run it through the real dispatcher, then
/// drain the outbox so the settlement orchestration runs.
async fn deliver_webhook(env: &TestEnv, payload: &[u8]) {
    let connector = mock_connector();
    let connector_impl =
        initialize_payment_connector(&connector).expect("mock connector init");

    let event = connector_impl
        .parse_event(&connector, payload, &HeaderMap::new())
        .expect("parse_event ok")
        .expect("an event was produced");

    handle_normalized_event(event, &connector, connector_impl.as_ref(), env.store().clone())
        .await
        .expect("dispatcher handled the event");

    env.run_outbox_and_orchestration().await;
}

async fn transaction_for_invoice(
    env: &TestEnv,
    invoice_id: InvoiceId,
) -> meteroid_store::domain::payment_transactions::PaymentTransaction {
    let txs = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list payment transactions");
    assert_eq!(txs.len(), 1, "expected exactly one payment transaction");
    txs.into_iter().next().unwrap().transaction
}

/// The persisted `next_action` JSONB column. The domain `PaymentTransaction`
/// intentionally ghosts this field to `None` (it's transient on the wire), so
/// to assert persistence/clearing we read the raw row.
async fn persisted_next_action(
    env: &TestEnv,
    tx_id: PaymentTransactionId,
) -> Option<serde_json::Value> {
    use diesel_models::payments::PaymentTransactionRow;
    let mut conn = env.conn().await;
    PaymentTransactionRow::get_by_id(&mut conn, tx_id, TENANT_ID)
        .await
        .expect("get payment transaction row")
        .next_action
}

/// Set up a card subscription whose first invoice is finalized but unpaid, and
/// return `(sub_id, invoice_id)`.
async fn unpaid_card_invoice(env: &TestEnv) -> (common_domain::ids::SubscriptionId, InvoiceId) {
    env.seed_payments().await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).is_finalized_unpaid();
    let invoice_id = invoices[0].id;
    (sub_id, invoice_id)
}

fn succeeded_payload(tx_id: PaymentTransactionId, external_id: &str) -> Vec<u8> {
    format!(
        r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"{ext}","amount":3500,"currency":"EUR"}}"#,
        tx = tx_id.as_base62(),
        ext = external_id,
    )
    .into_bytes()
}

// =============================================================================
// Async settlement: charge -> Pending -> webhook succeeded -> Settled + paid
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_async_charge_settles_via_webhook(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id) = unpaid_card_invoice(&env).await;

    // Async provider: the charge is accepted but settles later.
    env.set_mock_charge_behavior("pending").await;

    let (tx, next_action) = env
        .services()
        .complete_invoice_payment(TENANT_ID, invoice_id, CUST_UBER_PAYMENT_METHOD_ID, false)
        .await
        .expect("charge accepted");

    // Charge is pending: no inline action, transaction Pending, invoice unpaid.
    assert!(next_action.is_none());
    assert_eq!(tx.status, PaymentStatusEnum::Pending);
    let external_id = tx
        .provider_transaction_id
        .clone()
        .expect("async charge records a provider id for reconciliation");
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid();

    // Provider confirms the payment via webhook.
    deliver_webhook(&env, &succeeded_payload(tx.id, &external_id)).await;

    // Transaction settled, invoice paid.
    let settled = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(settled.status, PaymentStatusEnum::Settled);
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid();
}

// =============================================================================
// 3DS/SCA: charge -> requires_action -> next_action persisted; webhook clears it
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_requires_action_persists_then_clears_on_settlement(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id) = unpaid_card_invoice(&env).await;

    // Charge comes back needing 3DS/SCA.
    env.set_mock_charge_behavior("requires_action").await;

    let (tx, next_action) = env
        .services()
        .complete_invoice_payment(TENANT_ID, invoice_id, CUST_UBER_PAYMENT_METHOD_ID, true)
        .await
        .expect("charge accepted");

    // On-session caller gets a next_action to drive 3DS; tx stays Pending.
    assert!(next_action.is_some(), "on-session 3DS surfaces a next_action");
    assert_eq!(tx.status, PaymentStatusEnum::Pending);

    // The action is persisted on the transaction (so dunning/portal can resume).
    assert!(
        persisted_next_action(&env, tx.id).await.is_some(),
        "requires_action must be persisted on the pending transaction"
    );
    let external_id = transaction_for_invoice(&env, invoice_id)
        .await
        .provider_transaction_id
        .clone()
        .unwrap_or_default();

    // Customer completes 3DS; provider confirms via webhook.
    deliver_webhook(&env, &succeeded_payload(tx.id, &external_id)).await;

    let settled = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(settled.status, PaymentStatusEnum::Settled);
    assert!(
        persisted_next_action(&env, tx.id).await.is_none(),
        "next_action must be cleared once the charge reaches a terminal state"
    );
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid();
}

// =============================================================================
// Failure: charge -> Pending -> webhook failed -> Failed, invoice stays unpaid
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_async_charge_failure_via_webhook_leaves_invoice_unpaid(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, invoice_id) = unpaid_card_invoice(&env).await;

    env.set_mock_charge_behavior("pending").await;

    let (tx, _) = env
        .services()
        .complete_invoice_payment(TENANT_ID, invoice_id, CUST_UBER_PAYMENT_METHOD_ID, false)
        .await
        .expect("charge accepted");
    assert_eq!(tx.status, PaymentStatusEnum::Pending);

    let payload = format!(
        r#"{{"id":"evt_fail_{tx}","kind":"payment_failed","transaction_id":"{tx}","external_id":"pi_fail"}}"#,
        tx = tx.id.as_base62(),
    )
    .into_bytes();
    deliver_webhook(&env, &payload).await;

    let failed = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(failed.status, PaymentStatusEnum::Failed);
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid();
}
