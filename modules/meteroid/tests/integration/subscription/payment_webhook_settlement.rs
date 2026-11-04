//! End-to-end, mock-driven settlement tests against a real Postgres test DB.
//!
//! These cover the *asynchronous* payment paths the synchronous-success tests
//! in `payment_methods_config.rs` don't: a charge that comes back `Pending`
//! (SEPA/ACH-style) or `requires_action` (3DS/SCA) settles only when the
//! provider's webhook arrives. We drive the real production dispatcher
//! (`TestEnv::deliver_mock_webhook`) with events produced by the `MockConnector`'s
//! own `parse_event`, so the test exercises parse → consolidate → outbox →
//! settlement orchestration → invoice paid, exactly as the HTTP webhook route
//! does (minus signature verification, which is unit-tested per adapter).

use rstest::rstest;

use chrono::NaiveDate;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, test_env};

use meteroid_store::domain::PaymentStatusEnum;
use meteroid_store::domain::connectors::MockChargeBehavior;
use meteroid_store::domain::enums::{
    InvoicePaymentStatus, InvoiceStatusEnum, PaymentTypeEnum, RefundMode, ReversalKind,
};
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;

use common_domain::ids::{BaseId, InvoiceId, PaymentTransactionId};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};

async fn transaction_for_invoice(
    env: &TestEnv,
    invoice_id: InvoiceId,
) -> meteroid_store::domain::payment_transactions::PaymentTransaction {
    let mut txs: Vec<_> = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list payment transactions")
        .into_iter()
        .map(|t| t.transaction)
        .filter(|t| t.payment_type == PaymentTypeEnum::Payment)
        .collect();
    assert_eq!(txs.len(), 1, "expected exactly one payment transaction");
    txs.remove(0)
}

/// The REFUND children recorded against `parent_id`, oldest first.
async fn refunds_of(
    env: &TestEnv,
    parent_id: PaymentTransactionId,
) -> Vec<meteroid_store::domain::payment_transactions::PaymentTransaction> {
    use diesel_models::payments::PaymentTransactionRow;
    let mut conn = env.conn().await;
    PaymentTransactionRow::list_refunds_by_parent(&mut conn, parent_id, TENANT_ID)
        .await
        .expect("list refunds of a transaction")
        .into_iter()
        .map(Into::into)
        .collect()
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
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;

    // Async provider: the charge is accepted but settles later.
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;

    let (tx, next_action) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
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
    env.deliver_mock_webhook(&succeeded_payload(tx.id, &external_id))
        .await;

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
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;

    // Charge comes back needing 3DS/SCA.
    env.set_mock_charge_behavior(MockChargeBehavior::RequiresAction)
        .await;

    let (tx, next_action) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            true,
            None,
        )
        .await
        .expect("charge accepted");

    // On-session caller gets a next_action to drive 3DS; tx stays Pending.
    assert!(
        next_action.is_some(),
        "on-session 3DS surfaces a next_action"
    );
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
    env.deliver_mock_webhook(&succeeded_payload(tx.id, &external_id))
        .await;

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
async fn test_async_charge_failure_errors_invoice_and_schedules_dunning(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;

    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;

    let (tx, _) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect("charge accepted");
    assert_eq!(tx.status, PaymentStatusEnum::Pending);

    let payload = format!(
        r#"{{"id":"evt_fail_{tx}","kind":"payment_failed","transaction_id":"{tx}","external_id":"pi_fail"}}"#,
        tx = tx.id.as_base62(),
    )
    .into_bytes();
    env.deliver_mock_webhook(&payload).await;

    let failed = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(failed.status, PaymentStatusEnum::Failed);

    // No money moved, so the invoice stays a collectible document — but it is now
    // visibly errored rather than silently indistinguishable from a fresh unpaid one,
    // and the failure handler has put it on the dunning ladder.
    env.run_outbox_and_orchestration().await;
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .has_status(InvoiceStatusEnum::Finalized)
        .has_payment_status(InvoicePaymentStatus::Errored);

    let retries = env.pending_payment_retries(sub_id, invoice_id).await;
    assert_eq!(
        retries, 1,
        "a failed collection should schedule exactly one dunning retry"
    );
}

/// A stamped transaction id that was rolled back (replayed onto a newer row) settles the row
/// holding the provider payment id.
#[rstest]
#[tokio::test]
async fn test_success_webhook_for_a_rolled_back_transaction_settles_by_provider_id(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;

    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    env.services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect("charge accepted");
    let external_id = transaction_for_invoice(&env, invoice_id)
        .await
        .provider_transaction_id
        .clone()
        .expect("the pending charge carries the provider id");

    let rolled_back = PaymentTransactionId::new();
    env.deliver_mock_webhook(&succeeded_payload(rolled_back, &external_id))
        .await;

    assert_eq!(
        transaction_for_invoice(&env, invoice_id).await.status,
        PaymentStatusEnum::Settled
    );
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid();
}

/// A failed provider-hosted attempt (no saved method) is not a failed collection: no dunning.
#[rstest]
#[tokio::test]
async fn test_hosted_attempt_failure_keeps_invoice_payable_without_dunning(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;
    let invoice = env.get_invoices(sub_id).await.remove(0);

    let tx_id = PaymentTransactionId::new();
    {
        let mut conn = env.conn().await;
        diesel_models::payments::PaymentTransactionRowNew {
            id: tx_id,
            tenant_id: TENANT_ID,
            invoice_id: Some(invoice_id),
            provider_transaction_id: Some("tr_hosted".to_string()),
            amount: invoice.amount_due,
            currency: invoice.currency.clone(),
            payment_method_id: None,
            status: diesel_models::enums::PaymentStatusEnum::Pending,
            payment_type: diesel_models::enums::PaymentTypeEnum::Payment,
            error_type: None,
            processed_at: None,
            checkout_session_id: None,
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: None,
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            reversal_reason: None,
        }
        .insert(&mut conn)
        .await
        .expect("insert hosted attempt");
    }

    let payload = format!(
        r#"{{"id":"evt_hosted_{tx}","kind":"payment_failed","transaction_id":"{tx}","external_id":"tr_hosted"}}"#,
        tx = tx_id.as_base62(),
    )
    .into_bytes();
    env.deliver_mock_webhook(&payload).await;

    assert_eq!(
        transaction_for_invoice(&env, invoice_id).await.status,
        PaymentStatusEnum::Failed
    );
    env.run_outbox_and_orchestration().await;
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .has_status(InvoiceStatusEnum::Finalized)
        .has_payment_status(InvoicePaymentStatus::Unpaid);
    assert_eq!(
        env.pending_payment_retries(sub_id, invoice_id).await,
        0,
        "a hosted attempt the customer abandoned must not start dunning"
    );
}

/// Cancelled attempts are aborted, not declined: they must not consume dunning budget.
/// Three prior Cancelled attempts + the first genuine failure must still schedule the
/// first retry (counting Cancelled would read 4 attempts and exhaust the 3-slot ladder).
#[rstest]
#[tokio::test]
async fn test_cancelled_attempts_do_not_consume_dunning_budget(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;

    let mut conn = env.conn().await;
    for i in 0..3 {
        diesel_models::payments::PaymentTransactionRowNew {
            id: PaymentTransactionId::new(),
            tenant_id: TENANT_ID,
            invoice_id: Some(invoice_id),
            provider_transaction_id: Some(format!("pi_cancelled_{i}")),
            amount: 3500,
            currency: "EUR".to_string(),
            payment_method_id: Some(CUST_UBER_PAYMENT_METHOD_ID),
            status: diesel_models::enums::PaymentStatusEnum::Cancelled,
            payment_type: diesel_models::enums::PaymentTypeEnum::Payment,
            error_type: None,
            processed_at: None,
            checkout_session_id: None,
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: None,
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            reversal_reason: None,
        }
        .insert(&mut conn)
        .await
        .expect("seed cancelled attempt");
    }
    drop(conn);

    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    let (tx, _) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect("charge accepted despite cancelled history");
    assert_eq!(tx.status, PaymentStatusEnum::Pending);

    let payload = format!(
        r#"{{"id":"evt_fail_{tx}","kind":"payment_failed","transaction_id":"{tx}","external_id":"pi_fail_after_cancels"}}"#,
        tx = tx.id.as_base62(),
    )
    .into_bytes();
    env.deliver_mock_webhook(&payload).await;

    env.run_outbox_and_orchestration().await;
    let retries = env.pending_payment_retries(sub_id, invoice_id).await;
    assert_eq!(
        retries, 1,
        "the first genuine failure must schedule a retry; cancelled attempts don't count"
    );
}

/// A Cancelled off-session attempt was aborted, not declined: no Errored, no dunning.
#[rstest]
#[tokio::test]
async fn test_cancelled_attempt_keeps_invoice_payable_without_dunning(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;

    env.set_mock_charge_behavior(MockChargeBehavior::Cancelled)
        .await;
    let _ = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await;

    assert_eq!(
        transaction_for_invoice(&env, invoice_id).await.status,
        PaymentStatusEnum::Cancelled
    );
    env.run_outbox_and_orchestration().await;
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .has_status(InvoiceStatusEnum::Finalized)
        .has_payment_status(InvoicePaymentStatus::Unpaid);
    assert_eq!(
        env.pending_payment_retries(sub_id, invoice_id).await,
        0,
        "a cancelled attempt must not start dunning"
    );
}

// =============================================================================
// Money reversals: a payment lost to a dispute keeps its invoice paid; a failed
// collection (a debit the bank returned after it settled) reopens it.
// =============================================================================

/// GoCardless-style `late_failure_settled`: the bank reports the debit failed after it settled.
fn late_failure_payload(external_id: &str) -> Vec<u8> {
    format!(
        r#"{{"id":"evt_lf","kind":"payment_late_failure","external_id":"{ext}"}}"#,
        ext = external_id
    )
    .into_bytes()
}

/// A Stripe ACH/SEPA/Bacs debit returned after it succeeded, reported as a dispute whose reason
/// says the funds were missing: a failed collection.
fn bank_return_payload(
    external_id: &str,
    amount: i64,
    occurred_at: chrono::DateTime<chrono::Utc>,
) -> Vec<u8> {
    format!(
        r#"{{"id":"evt_br_{amt}","kind":"dispute_funds_withdrawn","external_id":"{ext}","amount":{amt},"currency":"EUR","reason":"insufficient_funds","occurred_at":"{at}"}}"#,
        ext = external_id,
        amt = amount,
        at = occurred_at.to_rfc3339(),
    )
    .into_bytes()
}

/// GoCardless-style full claw-back (`charged_back`). The event carries no metadata, so it
/// resolves the local transaction by the provider charge id — exactly the empty-metadata shape
/// real GoCardless sends.
fn reversed_payload(external_id: &str) -> Vec<u8> {
    format!(
        r#"{{"id":"evt_cb","kind":"payment_charged_back","external_id":"{ext}"}}"#,
        ext = external_id
    )
    .into_bytes()
}

/// Full claw-back pinning `occurred_at`, so a redelivery carries the SAME provider
/// timestamp as the original (a real redelivered webhook does), letting the
/// reversal-cycle high-water guard reject it.
fn reversed_payload_at(external_id: &str, occurred_at: chrono::DateTime<chrono::Utc>) -> Vec<u8> {
    format!(
        r#"{{"id":"evt_cb","kind":"payment_charged_back","external_id":"{ext}","occurred_at":"{at}"}}"#,
        ext = external_id,
        at = occurred_at.to_rfc3339(),
    )
    .into_bytes()
}

/// Stripe-style `charge.refunded`: the charge's cumulative refunded total plus the refund that
/// triggered it.
fn refunded_payload(external_id: &str, cumulative_minor: i64) -> Vec<u8> {
    format!(
        r#"{{"id":"evt_re_{amt}","kind":"payment_refunded","external_id":"{ext}","amount":{amt},"currency":"EUR","refunds":[{{"id":"mock_re_dash_{amt}","amount":{amt},"succeeded":true}}]}}"#,
        ext = external_id,
        amt = cumulative_minor,
    )
    .into_bytes()
}

fn failed_payload(tx_id: PaymentTransactionId, external_id: &str, evt: &str) -> Vec<u8> {
    format!(
        r#"{{"id":"evt_{evt}_{tx}","kind":"payment_failed","transaction_id":"{tx}","external_id":"{ext}"}}"#,
        tx = tx_id.as_base62(),
        ext = external_id,
    )
    .into_bytes()
}

/// Settle an invoice through the async settlement path (pending charge, then a
/// succeeded webhook) and return `(sub_id, invoice_id, tx_id, external_id, total)`.
async fn settled_invoice(
    env: &TestEnv,
) -> (
    common_domain::ids::SubscriptionId,
    InvoiceId,
    PaymentTransactionId,
    String,
    i64,
) {
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;
    let total = env.get_invoices(sub_id).await[0].total;

    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    let (tx, _) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect("charge accepted");
    let external_id = tx
        .provider_transaction_id
        .clone()
        .expect("async charge records a provider id");

    env.deliver_mock_webhook(&succeeded_payload(tx.id, &external_id))
        .await;
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);

    (sub_id, invoice_id, tx.id, external_id, total)
}

/// (a) settle → chargeback → transaction reversed and lost to the dispute; the invoice stays
/// paid and records the dispute.
#[rstest]
#[tokio::test]
async fn test_chargeback_reverses_settled_payment_and_keeps_invoice_paid(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let external_id = tx
        .provider_transaction_id
        .clone()
        .expect("a settled charge records a provider id");

    env.deliver_mock_webhook(&reversed_payload(&external_id))
        .await;

    let reversed = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(reversed.status, PaymentStatusEnum::Refunded);
    assert_eq!(
        reversed.amount_reversed, reversed.amount,
        "a full claw-back records the whole amount as involuntarily reversed"
    );
    assert_eq!(
        reversed.amount_disputed, reversed.amount,
        "all of it lost to the payer's claim"
    );
    assert_eq!(reversed.amount_reversed_excess, 0);
    assert_eq!(
        reversed.amount_refunded, 0,
        "nothing was given back voluntarily"
    );
    assert_eq!(reversed.reversal_kind, Some(ReversalKind::Chargeback));
    assert_eq!(reversed.reversal_reason.as_deref(), Some("mock_chargeback"));
    assert!(
        refunds_of(&env, reversed.id).await.is_empty(),
        "an involuntary reversal mutates the payment instead of writing a refund row"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);
    assert!(
        invoices[0].payment_disputed_at.is_some(),
        "the invoice records the dispute"
    );
}

/// (a') settle → late failure → the debit never really landed: the invoice reopens for its total.
#[rstest]
#[tokio::test]
async fn test_late_failure_reverses_settled_payment_and_reopens_invoice(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();

    env.deliver_mock_webhook(&late_failure_payload(&external_id))
        .await;

    let reversed = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(reversed.status, PaymentStatusEnum::Refunded);
    assert_eq!(reversed.amount_reversed, reversed.amount);
    assert_eq!(reversed.amount_disputed, 0);
    assert_eq!(
        reversed.reversal_kind,
        Some(ReversalKind::InsufficientFunds)
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_unpaid();
    assert_eq!(
        invoices[0].amount_due, total,
        "the invoice reopens for its full total once the collection failed"
    );
    assert!(invoices[0].payment_disputed_at.is_none());
}

/// (b) redelivering the chargeback must not double-reverse (idempotent).
#[rstest]
#[tokio::test]
async fn test_chargeback_redelivery_is_idempotent(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();

    env.deliver_mock_webhook(&reversed_payload(&external_id))
        .await;
    env.deliver_mock_webhook(&reversed_payload(&external_id))
        .await;

    let reversed = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(reversed.status, PaymentStatusEnum::Refunded);
    assert_eq!(reversed.amount_disputed, reversed.amount);
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);
}

/// (c) A provider-side (dashboard) refund is VOLUNTARY: it records REFUND children against the
/// payment and tracks the provider's cumulative figure on `amount_refunded`. The invoice keeps
/// its balance — what reduces an invoice is the Refund credit note, not the money movement —
/// so it stays paid even once the whole charge has been given back.
#[rstest]
#[tokio::test]
async fn test_partial_then_full_refund_tracks_net_amount_due(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();

    let partial = 1000;
    env.deliver_mock_webhook(&refunded_payload(&external_id, partial))
        .await;

    let after_partial = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(
        after_partial.status,
        PaymentStatusEnum::Settled,
        "a partial refund keeps the transaction settled"
    );
    assert_eq!(after_partial.amount_refunded, partial);
    assert_eq!(
        after_partial.amount_reversed, 0,
        "a refund is not a claw-back"
    );

    let refunds = refunds_of(&env, after_partial.id).await;
    assert_eq!(refunds.len(), 1, "the observed refund is recorded as a row");
    assert_eq!(refunds[0].amount, partial);
    assert_eq!(refunds[0].status, PaymentStatusEnum::Settled);
    assert_eq!(refunds[0].refund_mode, Some(RefundMode::Online));
    assert!(
        refunds[0].credit_note_id.is_none(),
        "a refund observed at the provider is not drawn on any credit note"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    assert_eq!(
        invoices[0].amount_due, 0,
        "a voluntary refund never reopens the invoice"
    );

    // The provider reports a new cumulative total: only the delta becomes a new row.
    env.deliver_mock_webhook(&refunded_payload(&external_id, total))
        .await;

    let after_full = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(
        after_full.status,
        PaymentStatusEnum::Refunded,
        "nothing is left in the merchant's hands"
    );
    assert_eq!(after_full.amount_refunded, total);

    let refunds = refunds_of(&env, after_full.id).await;
    assert_eq!(refunds.len(), 2);
    assert_eq!(
        refunds[1].amount,
        total - partial,
        "only the delta above the known refunds is recorded"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);
}

/// (d) A partially reversed invoice must stay collectable: a bank return reported as a partial
/// dispute is a failed collection, the live-payment fence (auto-charge / dunning) no longer
/// counts the returned part, and a new payment for the reopened balance is accepted.
#[rstest]
#[tokio::test]
async fn test_partially_reversed_balance_is_recollectable(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();

    let clawed_back = 1000;
    env.deliver_mock_webhook(&bank_return_payload(
        &external_id,
        clawed_back,
        chrono::Utc::now(),
    ))
    .await;
    let returned = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(
        (returned.amount_reversed, returned.amount_disputed),
        (clawed_back, 0)
    );
    assert_eq!(
        returned.reversal_kind,
        Some(ReversalKind::InsufficientFunds)
    );
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices[0].amount_due, clawed_back);

    // The re-collection fences (auto-charge orchestration, dunning retry) all
    // gate on this predicate: the partially reversed settlement must not own
    // the invoice while a reopened balance is outstanding.
    {
        use diesel_models::payments::PaymentTransactionRow;
        let mut conn = env.conn().await;
        let live = PaymentTransactionRow::exists_live_for_invoice(&mut conn, invoice_id, TENANT_ID)
            .await
            .expect("live-payment check");
        assert!(
            !live,
            "a partially reversed settlement must not block re-collection"
        );
    }

    // Collect the reopened balance (portal/manual/auto-charge path).
    let (tx2, _) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect("re-collection of the reopened balance is accepted");
    assert_eq!(tx2.status, PaymentStatusEnum::Settled);
    assert_eq!(
        tx2.amount, clawed_back,
        "the new charge collects only the reopened balance"
    );
    env.run_outbox_and_orchestration().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);
}

/// A 1000 partial dispute the previous release applied (no movement row, its event time kept as
/// the reversal mark, the invoice reopened as that release did) is re-emitted after the deploy by a
/// Mollie re-read and redelivered by Stripe: nothing is clawed back a second time.
#[rstest]
#[tokio::test]
async fn test_a_dispute_the_previous_release_applied_is_not_applied_again(
    #[future] test_env: TestEnv,
) {
    use diesel_async::SimpleAsyncConnection;
    let env = test_env.await;
    let (sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();
    let t1 = chrono::Utc::now() - chrono::Duration::hours(1);
    env.conn()
        .await
        .batch_execute(&format!(
            "UPDATE payment_transaction SET amount_reversed = 1000, reversal_kind = 'DISPUTE',
                    refunded_at = '{at}'
             WHERE id = '{tx}';
             UPDATE invoice SET amount_due = 1000, payment_status = 'PARTIALLY_PAID', paid_at = NULL
             WHERE id = '{invoice}';",
            at = t1.naive_utc(),
            tx = tx.id.as_uuid(),
            invoice = invoice_id.as_uuid(),
        ))
        .await
        .expect("the previous release's dispute");

    for _ in 0..2 {
        env.deliver_mock_webhook(&dispute_withdrawn_payload(&external_id, 1000, t1))
            .await;
        let payment = transaction_for_invoice(&env, invoice_id).await;
        assert_eq!(
            (payment.amount_reversed, payment.amount_disputed),
            (1000, 0),
            "the re-emitted dispute is already applied"
        );
        let invoices = env.get_invoices(sub_id).await;
        assert_eq!(
            (invoices[0].amount_due, invoices[0].payment_status),
            (1000, InvoicePaymentStatus::PartiallyPaid),
            "nothing more is owed"
        );
    }
}

/// A payment fails late, a replacement pays the invoice, then the merchant refunds 300 of the
/// replacement from the provider's dashboard (no credit note). Nothing was collected above the
/// invoice when the refund was made: the invoice stays paid, even after the reversal sweep.
#[rstest]
#[tokio::test]
async fn test_a_dashboard_refund_after_a_replaced_late_failure_keeps_the_invoice_paid(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
    let env = test_env.await;
    let (sub_id, invoice_id, tx, total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();
    env.deliver_mock_webhook(&late_failure_payload(&external_id))
        .await;
    assert_eq!(env.get_invoices(sub_id).await[0].amount_due, total);

    let (replacement, _) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect("the replacement payment");
    env.run_outbox_and_orchestration().await;
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid();

    let replacement_external = replacement
        .provider_transaction_id
        .clone()
        .expect("a provider id");
    env.deliver_mock_webhook(&refunded_payload(&replacement_external, 300))
        .await;
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);

    env.store()
        .reconcile_reversed_credit_purchases(100)
        .await
        .expect("sweep");
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);
}

/// The provider delivers a dispute's reinstatement before its withdrawal: the reinstatement is
/// held under the dispute and cancels the withdrawal when it arrives; redelivering either changes
/// nothing. The invoice never loses the money.
#[rstest]
#[tokio::test]
async fn test_a_reinstatement_delivered_before_its_withdrawal_cancels_it(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();
    let t1 = chrono::Utc::now();
    let t2 = t1 + chrono::Duration::minutes(5);

    for _ in 0..2 {
        env.deliver_mock_webhook(&dispute_reinstated_payload(&external_id, 1000, t2))
            .await;
        env.deliver_mock_webhook(&dispute_withdrawn_payload(&external_id, 1000, t1))
            .await;
        let payment = transaction_for_invoice(&env, invoice_id).await;
        assert_eq!(
            (payment.amount_reversed, payment.amount_disputed),
            (0, 0),
            "the dispute was won before its withdrawal was handled"
        );
        let invoices = env.get_invoices(sub_id).await;
        invoices.assert().invoice_at(0).is_finalized_paid();
        assert!(invoices[0].payment_disputed_at.is_none());
    }
}

/// A partial dispute leaves nothing to collect: the invoice stays paid and its settled payment
/// still owns it.
#[rstest]
#[tokio::test]
async fn test_partial_dispute_leaves_nothing_to_collect(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();

    env.deliver_mock_webhook(&dispute_withdrawn_payload(
        &external_id,
        1000,
        chrono::Utc::now(),
    ))
    .await;
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);
    let mut conn = env.conn().await;
    assert!(
        diesel_models::payments::PaymentTransactionRow::exists_live_for_invoice(
            &mut conn, invoice_id, TENANT_ID
        )
        .await
        .expect("live-payment check"),
        "the disputed payment still paid the invoice"
    );
}

// =============================================================================
// Dispute lifecycle: funds withdrawn claws back only the disputed amount and
// keeps the invoice paid; funds reinstated (merchant won) restores it.
// =============================================================================

fn dispute_withdrawn_payload(
    external_id: &str,
    amount: i64,
    occurred_at: chrono::DateTime<chrono::Utc>,
) -> Vec<u8> {
    format!(
        r#"{{"id":"evt_dw_{amt}","kind":"dispute_funds_withdrawn","external_id":"{ext}","amount":{amt},"currency":"EUR","occurred_at":"{at}"}}"#,
        ext = external_id,
        amt = amount,
        at = occurred_at.to_rfc3339(),
    )
    .into_bytes()
}

fn dispute_reinstated_payload(
    external_id: &str,
    amount: i64,
    occurred_at: chrono::DateTime<chrono::Utc>,
) -> Vec<u8> {
    format!(
        r#"{{"id":"evt_dr_{amt}","kind":"dispute_funds_reinstated","external_id":"{ext}","amount":{amt},"currency":"EUR","occurred_at":"{at}"}}"#,
        ext = external_id,
        amt = amount,
        at = occurred_at.to_rfc3339(),
    )
    .into_bytes()
}

/// GoCardless-style full reinstatement (`chargeback_cancelled`).
fn reinstated_payload(external_id: &str, occurred_at: chrono::DateTime<chrono::Utc>) -> Vec<u8> {
    format!(
        r#"{{"id":"evt_cbc","kind":"payment_reinstated","external_id":"{ext}","occurred_at":"{at}"}}"#,
        ext = external_id,
        at = occurred_at.to_rfc3339(),
    )
    .into_bytes()
}

/// (a) A partial dispute withdraws only the disputed amount (not the whole
/// charge) and keeps the invoice paid; winning it reinstates the funds; a
/// redelivered reinstatement has no double effect.
#[rstest]
#[tokio::test]
async fn test_partial_dispute_withdrawn_then_reinstated_recloses_invoice(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();

    let disputed = 1000;
    assert!(disputed < total);
    let t1 = chrono::Utc::now();
    let t2 = t1 + chrono::Duration::minutes(5);

    // Funds withdrawn for the disputed amount only.
    env.deliver_mock_webhook(&dispute_withdrawn_payload(&external_id, disputed, t1))
        .await;

    let after_withdraw = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(
        after_withdraw.status,
        PaymentStatusEnum::Settled,
        "a partial dispute must not reverse the whole transaction"
    );
    assert_eq!(after_withdraw.amount_reversed, disputed);
    assert_eq!(after_withdraw.amount_disputed, disputed);
    assert_eq!(after_withdraw.reversal_kind, Some(ReversalKind::Dispute));
    assert_eq!(
        after_withdraw.reversal_reason.as_deref(),
        Some("dispute.mock_dispute")
    );
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);
    assert!(invoices[0].payment_disputed_at.is_some());

    // Merchant wins: the withdrawn funds come back.
    env.deliver_mock_webhook(&dispute_reinstated_payload(&external_id, disputed, t2))
        .await;

    let after_reinstate = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(after_reinstate.status, PaymentStatusEnum::Settled);
    assert_eq!(after_reinstate.amount_reversed, 0);
    assert_eq!(after_reinstate.amount_disputed, 0);
    assert!(
        env.get_invoices(sub_id).await[0]
            .payment_disputed_at
            .is_none(),
        "nothing stays disputed"
    );
    assert!(
        after_reinstate.refunded_at.is_some(),
        "a full reinstatement keeps the reversal-cycle high-water mark so a redelivered original reversal (older timestamp) is rejected"
    );
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);

    // Redelivered reinstatement must not double-reduce anything.
    env.deliver_mock_webhook(&dispute_reinstated_payload(&external_id, disputed, t2))
        .await;
    let after_redelivery = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(after_redelivery.status, PaymentStatusEnum::Settled);
    assert_eq!(after_redelivery.amount_reversed, 0);
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);
}

/// (b) A dispute on a charge that was already partially refunded is tracked apart from it:
/// the voluntary refund sits on `amount_refunded`, the dispute claws back its own amount on
/// `amount_reversed`, and neither touches the invoice. Reinstating the dispute restores it,
/// and a redelivered reinstatement must not double-reduce.
#[rstest]
#[tokio::test]
async fn test_dispute_on_partially_refunded_charge_is_tracked_apart_and_reinstates_once(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();

    let refunded = 500;
    let disputed = 1000;
    let t1 = chrono::Utc::now();
    let t2 = t1 + chrono::Duration::minutes(5);
    let t3 = t1 + chrono::Duration::minutes(10);

    // Prior provider-side refund (cumulative total), then a dispute on top of it.
    env.deliver_mock_webhook(&refunded_payload(&external_id, refunded))
        .await;
    env.deliver_mock_webhook(&dispute_withdrawn_payload(&external_id, disputed, t2))
        .await;

    let after_withdraw = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(
        after_withdraw.amount_refunded, refunded,
        "the dispute must not be folded into the voluntary refunded total"
    );
    assert_eq!(after_withdraw.amount_reversed, disputed);
    assert_eq!(after_withdraw.reversal_kind, Some(ReversalKind::Dispute));
    assert_eq!(after_withdraw.amount_disputed, disputed);
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);

    // The dispute is won: the clawed-back part returns, the refund stays given back.
    env.deliver_mock_webhook(&dispute_reinstated_payload(&external_id, disputed, t3))
        .await;
    let after_reinstate = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(after_reinstate.amount_reversed, 0);
    assert_eq!(after_reinstate.amount_refunded, refunded);
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);

    // Redelivery of the same reinstatement event must be a no-op.
    env.deliver_mock_webhook(&dispute_reinstated_payload(&external_id, disputed, t3))
        .await;
    let after_redelivery = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(
        after_redelivery.amount_reversed, 0,
        "a redelivered reinstatement must not double-reduce the reversed total"
    );
    assert_eq!(after_redelivery.amount_refunded, refunded);
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).has_amount_due(0);
}

/// (c) GoCardless-style: a full chargeback flips the transaction to Refunded and keeps
/// the invoice paid; `chargeback_cancelled` restores the settlement. Redelivery is a no-op.
#[rstest]
#[tokio::test]
async fn test_chargeback_cancelled_restores_settlement_and_recloses_invoice(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();
    let t2 = chrono::Utc::now() + chrono::Duration::minutes(5);

    env.deliver_mock_webhook(&reversed_payload(&external_id))
        .await;
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    assert!(invoices[0].payment_disputed_at.is_some());

    // The bank cancels the chargeback: everything comes back.
    env.deliver_mock_webhook(&reinstated_payload(&external_id, t2))
        .await;

    let reinstated = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(
        reinstated.status,
        PaymentStatusEnum::Settled,
        "a cancelled chargeback must restore the settlement"
    );
    assert_eq!(reinstated.amount_reversed, 0);
    assert!(
        reinstated.refunded_at.is_some(),
        "a cancelled chargeback keeps the reversal-cycle high-water mark for redelivery protection"
    );
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);

    // Redelivery must not change anything.
    env.deliver_mock_webhook(&reinstated_payload(&external_id, t2))
        .await;
    let after_redelivery = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(after_redelivery.status, PaymentStatusEnum::Settled);
    assert_eq!(after_redelivery.amount_reversed, 0);
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
}

/// Count `payment_transaction.saved` events for `tx_id` currently on the
/// `webhook_out` queue (bound to every routing key). Reads are non-destructive
/// (vt=0), so calling before/after an action yields exactly what that action
/// emitted.
async fn payment_tx_saved_count(env: &TestEnv, tx_id: PaymentTransactionId) -> usize {
    let msgs = env
        .store()
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(200),
            MessageReadVtSec(0),
        )
        .await
        .expect("read webhook_out queue");

    msgs.into_iter()
        .filter_map(|m| m.message)
        .filter_map(|m| serde_json::from_value::<OutboxEvent>(m.0).ok())
        .filter(|e| {
            matches!(
                e,
                OutboxEvent::PaymentTransactionSaved(ev) if ev.payment_transaction_id == tx_id
            )
        })
        .count()
}

/// (d) C2: a redelivered ORIGINAL chargeback arriving AFTER a full reinstatement
/// must not re-claw the funds. Reinstatement moves the transaction back to
/// Settled but keeps `refunded_at` as a reversal-cycle high-water mark, so the
/// redelivered chargeback (its own, earlier timestamp) is rejected and the
/// invoice stays paid.
#[rstest]
#[tokio::test]
async fn test_redelivered_chargeback_after_reinstatement_does_not_reopen(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();
    let t1 = chrono::Utc::now();
    let t2 = t1 + chrono::Duration::minutes(5);

    // Original chargeback (full) at t1: lost to the dispute, the invoice stays paid.
    env.deliver_mock_webhook(&reversed_payload_at(&external_id, t1))
        .await;
    let reversed = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(reversed.status, PaymentStatusEnum::Refunded);
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();

    // Chargeback cancelled at t2 > t1: settlement restored, invoice re-closed.
    env.deliver_mock_webhook(&reinstated_payload(&external_id, t2))
        .await;
    let reinstated = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(reinstated.status, PaymentStatusEnum::Settled);
    assert_eq!(reinstated.amount_reversed, 0);
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);

    // The provider redelivers the ORIGINAL chargeback (same t1 timestamp) after
    // the reinstatement. It must be a no-op: no re-claw, invoice stays paid.
    env.deliver_mock_webhook(&reversed_payload_at(&external_id, t1))
        .await;

    let after_redelivery = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(
        after_redelivery.status,
        PaymentStatusEnum::Settled,
        "a redelivered original chargeback after reinstatement must not re-reverse"
    );
    assert_eq!(
        after_redelivery.amount_reversed, 0,
        "amount_reversed must stay 0 — the stale chargeback is rejected by the high-water guard"
    );
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);
}

/// (e) C1: a full reinstatement must not re-emit a `payment_transaction.saved`
/// event. Such an event carries the (Settled, `amount_reversed == 0`) shape that
/// the settled handler's first orchestration arm matches, which would re-run
/// `on_payment_transaction_settled` and re-apply a deferred plan change (gated
/// only on `pending_plan_version_id`, never cleared from the row).
///
/// Harness note: standing up a settled checkout transaction that carries a
/// deferred `pending_plan_version_id`, then reversing and reinstating it, is not
/// expressible through the current webhook/settlement seams, so this asserts the
/// mechanism directly — that reinstatement emits no settled-handler-routed event
/// (contrasted with the reversal path, which does).
#[rstest]
#[tokio::test]
async fn test_reinstatement_does_not_reemit_settled_event(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();
    let t2 = chrono::Utc::now() + chrono::Duration::minutes(5);

    // A full chargeback (like every reversal that moves money) emits a
    // payment_transaction.saved event — the positive control.
    env.deliver_mock_webhook(&reversed_payload(&external_id))
        .await;
    let baseline = payment_tx_saved_count(&env, tx.id).await;
    assert!(
        baseline >= 1,
        "the reversal path emits a payment_transaction.saved event"
    );

    // Cancel the chargeback: the reinstatement does its work synchronously and must
    // NOT emit a payment_transaction.saved event.
    env.deliver_mock_webhook(&reinstated_payload(&external_id, t2))
        .await;

    let after = payment_tx_saved_count(&env, tx.id).await;
    assert_eq!(
        after, baseline,
        "a full reinstatement must not re-emit a settled-handler-routed payment_transaction.saved event"
    );

    // ...and it still did its synchronous work: the invoice is paid again.
    let reinstated = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(reinstated.status, PaymentStatusEnum::Settled);
    assert_eq!(reinstated.amount_reversed, 0);
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);
}

// =============================================================================
// Out-of-order events: a bare failure event on a Settled transaction is always
// stale (real claw-backs arrive as amount-carrying refund/chargeback events and
// go through `reverse_transaction_tx`), so it must be a no-op — the invoice
// stays paid. Conversely a success arriving after a stale failure means the
// money was captured, so Failed → Settled is honored.
// =============================================================================

/// (a) settle, then deliver a stale/late `payment_failed` for the same tx: the
/// transaction stays Settled and the invoice stays Paid.
#[rstest]
#[tokio::test]
async fn test_stale_payment_failed_after_settlement_is_ignored(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx_id, external_id, _total) = settled_invoice(&env).await;

    // A late/out-of-order failure event (Stripe describes an earlier failed
    // attempt; delivery order is not guaranteed). Deliver it twice for good
    // measure — it must never claw back the settled funds.
    env.deliver_mock_webhook(&failed_payload(tx_id, &external_id, "stale1"))
        .await;
    env.deliver_mock_webhook(&failed_payload(tx_id, &external_id, "stale2"))
        .await;

    let tx = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(
        tx.status,
        PaymentStatusEnum::Settled,
        "a stale failure event must not reverse a settled transaction"
    );
    assert!(
        tx.refunded_at.is_none(),
        "a rejected transition must not stamp refunded_at"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);
}

/// (b) failed first, success after (out-of-order delivery of a captured charge):
/// the settlement wins and the invoice ends up paid.
#[rstest]
#[tokio::test]
async fn test_out_of_order_success_after_failure_settles(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;

    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    let (tx, _) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect("charge accepted");
    let external_id = tx.provider_transaction_id.clone().unwrap();

    env.deliver_mock_webhook(&failed_payload(tx.id, &external_id, "ooo"))
        .await;
    assert_eq!(
        transaction_for_invoice(&env, invoice_id).await.status,
        PaymentStatusEnum::Failed
    );

    // The success event arrives late: the money was captured, so it must win.
    env.deliver_mock_webhook(&succeeded_payload(tx.id, &external_id))
        .await;

    let settled = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(
        settled.status,
        PaymentStatusEnum::Settled,
        "an out-of-order success after a stale failure must settle"
    );
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid();
}

/// (c) a stale Settled orchestration event redelivered after a full failed collection
/// (pgmq at-least-once / unordered batch) recomputes amount_due == total; the
/// reopened invoice must stay Unpaid, not get relabeled PartiallyPaid.
#[rstest]
#[tokio::test]
async fn test_stale_settled_event_after_reversal_keeps_invoice_unpaid(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, _invoice_id, tx, total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();

    // Snapshot the Settled event as it sat on the queue before the reversal.
    let stale_event: meteroid_store::domain::outbox_event::PaymentTransactionEvent =
        tx.clone().into();

    // A full late failure reopens the invoice as Unpaid.
    env.deliver_mock_webhook(&late_failure_payload(&external_id))
        .await;
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_unpaid();

    // Redeliver the stale Settled event after the reversal was processed.
    env.services()
        .on_payment_transaction_settled(stale_event)
        .await
        .expect("stale settled event is handled idempotently");

    let invoices = env.get_invoices(sub_id).await;
    invoices
        .assert()
        .invoice_at(0)
        .has_payment_status(InvoicePaymentStatus::Unpaid);
    invoices.assert().invoice_at(0).has_amount_due(total);
}

// =============================================================================
// Credit notes vs. money in flight / later settlement
// =============================================================================

fn succeeded_payload_for(tx_id: PaymentTransactionId, external_id: &str, amount: i64) -> Vec<u8> {
    format!(
        r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"{ext}","amount":{amount},"currency":"EUR"}}"#,
        tx = tx_id.as_base62(),
        ext = external_id,
    )
    .into_bytes()
}

/// Remove the single LeetCode rate component mid-period, issuing the credit note the
/// amendment path decides on. Returns the credit note.
async fn amend_remove_rate_component(
    env: &TestEnv,
    sub_id: common_domain::ids::SubscriptionId,
    invoice_id: InvoiceId,
) -> meteroid_store::domain::credit_notes::CreditNote {
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::credit_notes::CreditNoteInterface;

    let rate = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|c| c.name == "Subscription Rate")
        .expect("Subscription Rate component");

    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![],
                    removed: vec![rate.id],
                },
                add_on_changes: AddOnChanges::default(),
            },
            NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(),
        )
        .await
        .expect("apply amendment failed");

    let mut credit_notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list credit notes");
    assert_eq!(credit_notes.len(), 1, "exactly one credit note expected");
    credit_notes.remove(0)
}

/// A DebtCancellation credit note on an unpaid invoice must survive the later
/// settlement of the remaining balance: paying `total - credit` closes the invoice
/// (the settle recompute nets the cancelled debt out; it must not re-bill it).
#[rstest]
#[tokio::test]
async fn test_debt_cancellation_survives_later_settlement(#[future] test_env: TestEnv) {
    use meteroid_store::domain::enums::CreditType;

    let env = test_env.await;
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;

    let cn = amend_remove_rate_component(&env, sub_id, invoice_id).await;
    assert_eq!(cn.credit_type, CreditType::DebtCancellation);
    let credit = cn.total.abs();
    assert!(credit > 0 && credit < 3500);

    let invoices = env.get_invoices(sub_id).await;
    invoices
        .assert()
        .invoice_at(0)
        .has_payment_status(InvoicePaymentStatus::Unpaid)
        .has_amount_due(3500 - credit);

    // Pay the reduced balance with the (synchronous) card.
    let (tx, _) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect("charge accepted");
    assert_eq!(tx.status, PaymentStatusEnum::Settled);
    assert_eq!(
        tx.amount,
        3500 - credit,
        "only the remaining debt is charged"
    );
    env.run_outbox_and_orchestration().await;

    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid()
        .has_amount_due(0);
}

/// An amendment while a direct debit is in flight must NOT cancel debt (the debit
/// collects the full amount regardless); it credits the customer balance, and the
/// later settlement closes the invoice at its full total.
#[rstest]
#[tokio::test]
async fn test_amendment_with_in_flight_debit_credits_balance(#[future] test_env: TestEnv) {
    use meteroid_store::domain::enums::CreditType;

    let env = test_env.await;
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;
    env.seed_uber_sepa_payment_method().await;
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;

    let (tx, _) = env
        .services()
        .complete_invoice_payment(TENANT_ID, invoice_id, CUST_UBER_SEPA_METHOD_ID, false, None)
        .await
        .expect("debit accepted");
    assert_eq!(tx.status, PaymentStatusEnum::Pending);
    let external_id = tx.provider_transaction_id.clone().expect("provider id");

    let balance_before = env.invoice_credit_balance_minor(CUST_UBER_ID).await;

    let cn = amend_remove_rate_component(&env, sub_id, invoice_id).await;
    assert_eq!(cn.credit_type, CreditType::CreditToBalance);
    assert!(cn.credited_amount_cents > 0);
    let balance_after = env.invoice_credit_balance_minor(CUST_UBER_ID).await;
    assert_eq!(balance_after - balance_before, cn.credited_amount_cents);

    // The in-flight debit still collects the full amount; nothing was cancelled.
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .has_amount_due(3500);

    env.deliver_mock_webhook(&succeeded_payload_for(tx.id, &external_id, 3500))
        .await;

    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid()
        .has_amount_due(0);
    // The credit stays on the balance: no money is eaten by the settlement.
    assert_eq!(
        env.invoice_credit_balance_minor(CUST_UBER_ID).await,
        balance_after
    );
}

/// Same rule keyed on the invoice's own `Processing` marker (accepted async debit
/// recorded at checkout), independently of a live transaction row.
#[rstest]
#[tokio::test]
async fn test_amendment_on_processing_invoice_credits_balance(#[future] test_env: TestEnv) {
    use diesel_models::invoices::InvoiceRow;
    use meteroid_store::domain::enums::CreditType;

    let env = test_env.await;
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;
    {
        let mut conn = env.conn().await;
        InvoiceRow::apply_payment_status(
            &mut conn,
            invoice_id,
            TENANT_ID,
            diesel_models::enums::InvoicePaymentStatus::Processing,
            None,
        )
        .await
        .expect("mark invoice processing");
    }

    let cn = amend_remove_rate_component(&env, sub_id, invoice_id).await;
    assert_eq!(cn.credit_type, CreditType::CreditToBalance);
    assert!(cn.credited_amount_cents > 0);
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .has_amount_due(3500);
}

// =============================================================================
// Existing direct-debit mandate at checkout: activate at acceptance, not at
// settlement (consistency with the activation path and the hosted flow).
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_selfserve_checkout_with_existing_mandate_activates_in_flight(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::domain::CreateCheckoutSession;
    use meteroid_store::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutType};
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

    let env = test_env.await;
    env.seed_payments().await;
    env.seed_uber_sepa_payment_method().await;
    // Direct debit never settles inline: accepted now, confirmed by webhook later.
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;

    let session = env
        .store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID, // $35/month, no trial
            billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap()),
            billing_day_anchor: Some(1),
            net_terms: None,
            trial_duration_days: None,
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            payment_methods_config: None,
            components: None,
            add_ons: None,
            coupon_code: None,
            coupon_ids: vec![],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: CheckoutType::SelfServe,
            subscription_id: None,
            change_date: None,
            success_url: None,
            cancel_url: None,
        })
        .await
        .expect("create checkout session");

    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_SEPA_METHOD_ID,
            3500,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("checkout completes on an accepted debit");

    let (sub_id, tx) = match result {
        CheckoutCompletionResult::TopUpCompleted { .. } => {
            panic!("subscription checkout returned a top-up")
        }
        CheckoutCompletionResult::Completed {
            subscription_id,
            transaction,
        } => (subscription_id, transaction.expect("the accepted debit")),
        CheckoutCompletionResult::AwaitingPayment { .. } => {
            panic!("an accepted debit on a saved mandate must activate at acceptance")
        }
    };
    assert_eq!(tx.status, PaymentStatusEnum::Pending);

    // Customer has access; the invoice is a real document with money in flight.
    env.get_subscription(sub_id).await.assert().is_active();
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_processing()
        .has_amount_due(3500);
    let invoice_id = invoices[0].id;

    // The charge is linked to the invoice and the session is done.
    let linked = transaction_for_invoice(&env, invoice_id).await;
    assert_eq!(linked.id, tx.id);
    assert_eq!(linked.status, PaymentStatusEnum::Pending);
    assert!(
        env.store()
            .get_checkout_session(TENANT_ID, session.id)
            .await
            .expect("session")
            .is_completed()
    );

    // Settlement closes the invoice through the invoice path (no second materialization).
    let external_id = linked.provider_transaction_id.clone().expect("provider id");
    env.deliver_mock_webhook(&succeeded_payload(tx.id, &external_id))
        .await;

    assert_eq!(
        transaction_for_invoice(&env, invoice_id).await.status,
        PaymentStatusEnum::Settled
    );
    env.get_invoices(sub_id)
        .await
        .assert()
        .has_count(1)
        .invoice_at(0)
        .is_finalized_paid()
        .has_amount_due(0);
}

// =============================================================================
// Deferred materialization prices at the charge's date, not webhook day
// =============================================================================

/// A checkout charge accepted on day D and settled on day D+2 must rebuild the
/// subscription/invoice as of D (the day the displayed amount was computed and
/// the customer accepted it) — otherwise day-sensitive proration drifts and the
/// amount guard rejects the webhook with the money already collected.
#[rstest]
#[tokio::test]
async fn test_deferred_checkout_materialization_prices_at_charge_date(#[future] test_env: TestEnv) {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use meteroid_store::domain::CreateCheckoutSession;
    use meteroid_store::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutType};
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

    let env = test_env.await;
    env.seed_payments().await;
    // Card that reports "processing": not an accepted debit, so nothing is
    // materialized until the settlement webhook (the deferred path).
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;

    let session = env
        .store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            billing_start_date: None, // priced "today" at confirm time
            billing_day_anchor: None,
            net_terms: None,
            trial_duration_days: None,
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            payment_methods_config: None,
            components: None,
            add_ons: None,
            coupon_code: None,
            coupon_ids: vec![],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: CheckoutType::SelfServe,
            subscription_id: None,
            change_date: None,
            success_url: None,
            cancel_url: None,
        })
        .await
        .expect("create checkout session");

    let tx = match env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3500,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("checkout accepted")
    {
        CheckoutCompletionResult::TopUpCompleted { .. } => {
            panic!("subscription checkout returned a top-up")
        }
        CheckoutCompletionResult::AwaitingPayment { transaction, .. } => transaction,
        CheckoutCompletionResult::Completed { .. } => panic!("card processing must defer"),
    };
    let external_id = tx.provider_transaction_id.clone().expect("provider id");

    // Simulate two days passing before the bank confirms: the charge keeps its
    // original (priced) creation day.
    let priced_on = (chrono::Utc::now() - chrono::Duration::days(2)).date_naive();
    {
        use diesel_models::schema::payment_transaction::dsl as pt;
        let mut conn = env.conn().await;
        diesel::update(pt::payment_transaction.filter(pt::id.eq(tx.id)))
            .set(pt::created_at.eq(chrono::Utc::now() - chrono::Duration::days(2)))
            .execute(&mut conn)
            .await
            .expect("backdate transaction");
    }

    env.deliver_mock_webhook(&succeeded_payload(tx.id, &external_id))
        .await;

    let session = env
        .store()
        .get_checkout_session(TENANT_ID, session.id)
        .await
        .expect("session");
    assert!(
        session.is_completed(),
        "settlement materialized the checkout"
    );
    let sub_id = session
        .subscription_id
        .expect("subscription created at settlement");
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(
        sub.start_date, priced_on,
        "subscription starts on the priced day"
    );
    assert_eq!(sub.billing_start_date, Some(priced_on));
    assert_eq!(sub.current_period_start, priced_on);

    env.get_invoices(sub_id)
        .await
        .assert()
        .has_count(1)
        .invoice_at(0)
        .is_finalized_paid()
        .has_amount_due(0);
}

// =============================================================================
// Disabled connector: an auto-charge refuses cleanly, and a settlement still lands
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_disabled_connector_starts_no_charge_but_still_settles(#[future] test_env: TestEnv) {
    use common_domain::actor::Actor;
    use meteroid_store::errors::StoreError;
    use meteroid_store::repositories::connectors::ConnectorsInterface;

    let env = test_env.await;
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;
    let charge = || {
        env.services().complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
    };

    // Disabled: nothing new starts, and the refusal is final rather than retryable.
    env.store()
        .disable_connector(Actor::System, TENANT_ID, MOCK_CONNECTOR_ID)
        .await
        .unwrap();
    let err = charge()
        .await
        .expect_err("a disabled connector starts no charge");
    assert!(matches!(
        err.current_context(),
        StoreError::ConnectorNotActive(_)
    ));
    assert!(
        env.store()
            .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
            .await
            .unwrap()
            .is_empty(),
        "the refused charge leaves no transaction behind"
    );
    // The queued-charge fallback (invoice email with a payment link) succeeds.
    env.services()
        .fall_back_to_manual_payment(TENANT_ID, invoice_id)
        .await
        .unwrap();

    // Re-enabled, a charge goes out; disabled again while it is still pending.
    env.store()
        .enable_connector(Actor::System, TENANT_ID, MOCK_CONNECTOR_ID)
        .await
        .unwrap();
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    let (tx, _) = charge().await.expect("charge accepted");
    let external_id = tx.provider_transaction_id.clone().unwrap();
    env.store()
        .disable_connector(Actor::System, TENANT_ID, MOCK_CONNECTOR_ID)
        .await
        .unwrap();

    // The in-flight payment still settles.
    env.deliver_mock_webhook(&succeeded_payload(tx.id, &external_id))
        .await;
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid();
}

// =============================================================================
// Revoked key: the charge fails promptly and flags the connector
// =============================================================================

/// The charge holds a lock on the connector while it runs; recording the rejection must not wait
/// on that lock, or the charge hangs.
#[rstest]
#[tokio::test]
async fn test_rejected_credentials_during_a_charge_flag_the_connector(#[future] test_env: TestEnv) {
    use meteroid_store::repositories::connectors::ConnectorsInterface;

    let env = test_env.await;
    let (_sub_id, invoice_id) = env.unpaid_card_invoice().await;
    env.set_mock_charge_behavior(MockChargeBehavior::RejectedCredentials)
        .await;

    let charge = env.services().complete_invoice_payment(
        TENANT_ID,
        invoice_id,
        CUST_UBER_PAYMENT_METHOD_ID,
        false,
        None,
    );
    let result = tokio::time::timeout(std::time::Duration::from_secs(20), charge)
        .await
        .expect("the charge must not hang on its own connector lock");
    assert!(result.is_err(), "a rejected key fails the charge");

    let connector = env
        .store()
        .get_connector_with_data(MOCK_CONNECTOR_ID, TENANT_ID)
        .await
        .unwrap();
    assert!(
        connector
            .credentials_error()
            .as_ref()
            .is_some_and(|e| e.message.contains("rejected")),
        "the rejection is recorded: {:?}",
        connector.credentials_error()
    );
    assert!(
        env.store()
            .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
            .await
            .unwrap()
            .is_empty(),
        "the failed charge leaves no transaction behind"
    );
}
