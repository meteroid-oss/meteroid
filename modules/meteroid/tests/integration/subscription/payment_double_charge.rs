//! Money-state correctness tests for the multi-provider payment paths.
//!
//! Covers the two ways a naive implementation double-counts money:
//!   (a) re-completing an `AwaitingPayment` checkout while a charge is in flight
//!       (3DS `RequiresAction`→Pending is the common card case) — must NOT issue
//!       a second provider charge, and
//!   (b) a redelivered `PaymentTransactionSettled` event (pgmq commit-then-delete
//!       window) — must be a no-op, never driving a paid invoice negative.

use chrono::NaiveDate;
use rstest::rstest;

use common_domain::ids::CheckoutSessionId;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, TestEnv, subscription, test_env};

use meteroid_store::domain::CreateCheckoutSession;
use meteroid_store::domain::PaymentStatusEnum;
use meteroid_store::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutType};
use meteroid_store::domain::outbox_event::PaymentTransactionEvent;
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;

/// Count ALL payment_transaction rows tied to a checkout session (any status).
/// Exactly one proves a single charge was attempted despite two completions.
async fn count_txs_for_session(env: &TestEnv, session_id: CheckoutSessionId) -> i64 {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::payment_transaction::dsl as pt;

    let mut conn = env.conn().await;
    pt::payment_transaction
        .filter(pt::checkout_session_id.eq(session_id))
        .filter(pt::tenant_id.eq(TENANT_ID))
        .count()
        .get_result(&mut conn)
        .await
        .expect("count payment transactions for session")
}

// =============================================================================
// (a) Re-completing an AwaitingPayment SelfServe checkout charges exactly once
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_selfserve_double_completion_charges_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    // Async provider: the charge is accepted but settles later, so the session
    // lands in AwaitingPayment (the state can_complete() allows re-entry from).
    env.set_mock_charge_behavior("pending").await;

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
        })
        .await
        .expect("create checkout session");

    // First completion: charge accepted, awaiting async settlement.
    let first = env
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
        .expect("first completion");

    let tx1 = match first {
        CheckoutCompletionResult::AwaitingPayment { transaction, .. } => {
            assert_eq!(transaction.status, PaymentStatusEnum::Pending);
            transaction
        }
        CheckoutCompletionResult::Completed { .. } => {
            panic!("async charge should leave the checkout AwaitingPayment")
        }
    };

    assert_eq!(
        count_txs_for_session(&env, session.id).await,
        1,
        "first completion creates exactly one transaction"
    );

    // Second completion while the charge is still in flight: MUST return the
    // existing transaction, not issue a fresh charge (which would mint a new
    // idempotency key `charge:{id}` → a real second charge at the provider).
    let second = env
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
        .expect("second completion is idempotent, not an error");

    let tx2 = match second {
        CheckoutCompletionResult::AwaitingPayment { transaction, .. } => transaction,
        CheckoutCompletionResult::Completed { .. } => {
            panic!("re-completion must not activate/complete a still-pending checkout")
        }
    };

    assert_eq!(
        tx1.id, tx2.id,
        "re-completion returns the SAME transaction, not a new charge"
    );
    assert_eq!(
        tx1.provider_transaction_id, tx2.provider_transaction_id,
        "no second provider charge (same external id)"
    );
    assert_eq!(
        count_txs_for_session(&env, session.id).await,
        1,
        "still exactly one transaction after the second completion"
    );
}

// =============================================================================
// (b) A redelivered settled event is a no-op: invoice stays Paid, amount_due 0
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_redelivered_settlement_keeps_invoice_paid(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    // Card subscription whose first invoice is finalized but unpaid.
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

    // Synchronous success: transaction settles and an outbox settle event is queued.
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

    // Drain the outbox: the settlement consumer marks the invoice paid.
    env.run_outbox_and_orchestration().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    let total = invoices[0].total;
    assert_eq!(invoices[0].amount_due, 0, "fully paid invoice has zero due");

    // Reconstruct the settle event and redeliver it TWICE (simulating pgmq
    // at-least-once redelivery in the commit-then-delete window).
    let settled_tx = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list payment transactions")
        .into_iter()
        .next()
        .expect("one settled transaction")
        .transaction;
    let event: PaymentTransactionEvent = settled_tx.into();

    for _ in 0..2 {
        env.services()
            .on_payment_transaction_settled(event.clone())
            .await
            .expect("redelivered settle event handled");
    }

    // The invoice must be unchanged: still Paid, amount_due still 0 (never driven
    // negative, never flipped back to PartiallyPaid).
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(
        invoices[0].amount_due, 0,
        "redelivery must not drive amount_due negative"
    );
    assert_eq!(invoices[0].total, total, "total unchanged");
    invoices.assert().invoice_at(0).is_finalized_paid();
}

// =============================================================================
// (a2) Re-completing an AwaitingPayment SubscriptionActivation checkout charges
//      once. This path differs from self-serve: the charge is linked to the
//      invoice, so the transaction is tagged with the session id after the fact
//      (set_checkout_session_id) for the idempotency guard to find it.
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_activation_double_completion_charges_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    // Async provider: the activation charge is accepted but settles later, so
    // the session lands in AwaitingPayment.
    env.set_mock_charge_behavior("pending").await;

    // An OnCheckout subscription auto-creates a SubscriptionActivation session.
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month, no trial
        .start_date(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap())
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    let session = env
        .store()
        .get_checkout_session_by_subscription(TENANT_ID, sub_id)
        .await
        .expect("activation checkout session auto-created");
    assert_eq!(session.checkout_type, CheckoutType::SubscriptionActivation);

    let first = env
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
        .expect("first activation completion");

    let tx1 = match first {
        CheckoutCompletionResult::AwaitingPayment { transaction, .. } => {
            assert_eq!(transaction.status, PaymentStatusEnum::Pending);
            transaction
        }
        CheckoutCompletionResult::Completed { .. } => {
            panic!("async charge should leave the activation checkout AwaitingPayment")
        }
    };

    assert_eq!(
        count_txs_for_session(&env, session.id).await,
        1,
        "activation completion tags exactly one transaction with the session"
    );

    // Second completion while the charge is in flight: the guard finds the
    // tagged transaction and returns it, rather than charging again.
    let second = env
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
        .expect("second activation completion is idempotent, not an error");

    let tx2 = match second {
        CheckoutCompletionResult::AwaitingPayment { transaction, .. } => transaction,
        CheckoutCompletionResult::Completed { .. } => {
            panic!("re-completion must not activate a still-pending checkout")
        }
    };

    assert_eq!(
        tx1.id, tx2.id,
        "re-completion returns the SAME transaction, not a new charge"
    );
    assert_eq!(
        tx1.provider_transaction_id, tx2.provider_transaction_id,
        "no second provider charge (same external id)"
    );
    assert_eq!(
        count_txs_for_session(&env, session.id).await,
        1,
        "still exactly one transaction after the second completion"
    );
}

// =============================================================================
// (c) Reconciliation of a transaction the provider still reports as Pending is a
//     safe no-op: it must never cancel a row that carries a stored provider id
//     (cancellation is terminal and would drop a later settlement webhook).
//     The Succeeded/Unknown branch decisions are covered by the unit tests in
//     services/payment/reconcile.rs (the mock connector always reports Pending).
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_reconcile_pending_is_a_safe_noop(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    // Async charge: the resulting Pending transaction carries a provider id
    // (backfilled at charge time), making it eligible for the reconcile sweep.
    env.set_mock_charge_behavior("pending").await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_unpaid();
    let invoice_id = invoices[0].id;

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
        .expect("async charge accepted");
    assert_eq!(tx.status, PaymentStatusEnum::Pending);
    let provider_id = tx
        .provider_transaction_id
        .clone()
        .expect("async charge backfills a provider id");

    // Provider still reports Pending → reconcile changes nothing.
    env.services()
        .reconcile_pending_transaction(tx.id, TENANT_ID)
        .await
        .expect("reconcile is a safe no-op while the provider is still processing");

    let after = env
        .store()
        .get_payment_tx_by_provider_transaction_id(TENANT_ID, &provider_id)
        .await
        .expect("lookup by provider id")
        .expect("transaction still present");
    assert_eq!(
        after.status,
        PaymentStatusEnum::Pending,
        "reconcile must leave a still-processing transaction Pending, not cancel it"
    );
}
