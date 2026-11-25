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
use meteroid_store::domain::connectors::MockChargeBehavior;
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
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;

    let session = env
        .store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID, // $35/month, no trial
            option: None,
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
        CheckoutCompletionResult::TopUpCompleted { .. } => {
            panic!("subscription checkout returned a top-up")
        }
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
        CheckoutCompletionResult::TopUpCompleted { .. } => {
            panic!("subscription checkout returned a top-up")
        }
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
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;

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
        CheckoutCompletionResult::TopUpCompleted { .. } => {
            panic!("subscription checkout returned a top-up")
        }
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
        CheckoutCompletionResult::TopUpCompleted { .. } => {
            panic!("subscription checkout returned a top-up")
        }
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
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;

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

fn self_serve_session(tenant_customer: common_domain::ids::CustomerId) -> CreateCheckoutSession {
    CreateCheckoutSession {
        tenant_id: TENANT_ID,
        customer_id: tenant_customer,
        plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
        option: None,
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
    }
}

async fn session_transactions(
    env: &TestEnv,
    session_id: CheckoutSessionId,
) -> Vec<diesel_models::payments::PaymentTransactionRow> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::payment_transaction::dsl as pt;

    let mut conn = env.conn().await;
    pt::payment_transaction
        .filter(pt::checkout_session_id.eq(session_id))
        .filter(pt::tenant_id.eq(TENANT_ID))
        .select(diesel_models::payments::PaymentTransactionRow::as_select())
        .load(&mut conn)
        .await
        .expect("load session transactions")
}

/// The provider settled the charge (webhook) but the session is not fulfilled yet: a re-click
/// must return the settled charge, not issue a second one.
#[rstest]
#[tokio::test]
async fn test_recompletion_after_settlement_before_fulfillment_charges_once(
    #[future] test_env: TestEnv,
) {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::payment_transaction::dsl as pt;

    let env = test_env.await;
    env.seed_payments().await;
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;

    let session = env
        .store()
        .create_checkout_session(self_serve_session(CUST_UBER_ID))
        .await
        .expect("create checkout session");

    let complete = || {
        env.services().complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3500,
            "EUR".to_string(),
            None,
        )
    };

    let CheckoutCompletionResult::AwaitingPayment { transaction, .. } =
        complete().await.expect("first completion")
    else {
        panic!("async charge should leave the checkout AwaitingPayment")
    };

    // Settled at the provider, fulfillment (the outbox resolve) not run yet.
    let mut conn = env.conn().await;
    diesel::update(pt::payment_transaction.filter(pt::id.eq(transaction.id)))
        .set(pt::status.eq(diesel_models::enums::PaymentStatusEnum::Settled))
        .execute(&mut conn)
        .await
        .expect("settle transaction");
    drop(conn);

    let CheckoutCompletionResult::AwaitingPayment {
        transaction: again, ..
    } = complete().await.expect("re-completion is idempotent")
    else {
        panic!("re-completion must not charge again while the settled charge awaits fulfillment")
    };
    assert_eq!(again.id, transaction.id);
    assert_eq!(count_txs_for_session(&env, session.id).await, 1);
}

/// A plan-change charge that settles asynchronously is linked to its checkout session, so a
/// re-click is fenced instead of creating a second adjustment invoice and charge.
#[rstest]
#[tokio::test]
async fn test_plan_change_pending_recompletion_charges_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_checkout()
        .trial_days(7)
        .auto_charge()
        .create(env.services())
        .await;

    let mut conn = env.conn().await;
    env.services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            9900,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("initial checkout");
    drop(conn);

    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;

    let sub = env.get_subscription(sub_id).await;
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            PLAN_VERSION_PRO_ID,
            sub.customer_id,
            None,
            NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(),
        )
        .await
        .expect("create plan change session");

    let complete = || {
        env.services().complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            1290,
            "EUR".to_string(),
            None,
        )
    };

    let CheckoutCompletionResult::AwaitingPayment { transaction, .. } =
        complete().await.expect("first completion")
    else {
        panic!("async charge should leave the plan change AwaitingPayment")
    };
    let CheckoutCompletionResult::AwaitingPayment {
        transaction: again, ..
    } = complete().await.expect("re-completion is idempotent")
    else {
        panic!("re-completion must not complete a pending plan change")
    };

    assert_eq!(again.id, transaction.id);
    assert_eq!(count_txs_for_session(&env, session.id).await, 1);
    // The adjustment invoice is created when the charge settles, not before.
    env.get_invoices(sub_id).await.assert().has_count(1);
}

/// A cancelled checkout charge is recorded (as a declined one is), so the attempt is visible
/// and a retry starts from a clean, released session.
#[rstest]
#[tokio::test]
async fn test_cancelled_checkout_charge_is_recorded(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.set_mock_charge_behavior(MockChargeBehavior::Cancelled)
        .await;

    let session = env
        .store()
        .create_checkout_session(self_serve_session(CUST_UBER_ID))
        .await
        .expect("create checkout session");

    let err = env
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
        .expect_err("a cancelled charge does not complete the checkout");
    assert!(matches!(
        err.current_context(),
        meteroid_store::errors::StoreError::PaymentError(_)
    ));

    let txs = session_transactions(&env, session.id).await;
    assert_eq!(txs.len(), 1);
    assert_eq!(
        txs[0].status,
        diesel_models::enums::PaymentStatusEnum::Cancelled
    );
}

/// A checkout charge whose answer never came back was committed before it was sent: a re-click
/// is fenced instead of charging again, and the re-send under its key completes the checkout.
#[rstest]
#[tokio::test]
async fn test_checkout_charge_without_an_answer_is_resent_not_charged_again(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::domain::checkout_sessions::CheckoutSessionStatus;

    let env = test_env.await;
    env.seed_payments().await;
    env.set_mock_charge_behavior(MockChargeBehavior::Unknown)
        .await;

    let session = env
        .store()
        .create_checkout_session(self_serve_session(CUST_UBER_ID))
        .await
        .expect("create checkout session");
    let complete = || {
        env.services().complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3500,
            "EUR".to_string(),
            None,
        )
    };

    let CheckoutCompletionResult::AwaitingPayment { transaction, .. } = complete()
        .await
        .expect("an unanswered charge is in flight, not an error")
    else {
        panic!("an unanswered charge leaves the checkout awaiting its payment")
    };
    assert_eq!(transaction.status, PaymentStatusEnum::Pending);
    assert!(transaction.provider_transaction_id.is_none());

    // The provider would now take a second charge: the re-click must not send one.
    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    let CheckoutCompletionResult::AwaitingPayment {
        transaction: again, ..
    } = complete().await.expect("re-completion is fenced")
    else {
        panic!("re-completion must wait for the unanswered charge")
    };
    assert_eq!(again.id, transaction.id);
    let rows = session_transactions(&env, session.id).await;
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].provider_transaction_id.is_none(),
        "nothing was sent again"
    );

    env.services()
        .resume_unsent_charge(transaction.id, TENANT_ID)
        .await
        .expect("resume the unsent charge");
    env.run_outbox_and_orchestration().await;

    let rows = session_transactions(&env, session.id).await;
    assert_eq!(rows.len(), 1, "the re-send is the same attempt");
    assert_eq!(
        rows[0].status,
        diesel_models::enums::PaymentStatusEnum::Settled
    );
    let invoice_id = rows[0]
        .invoice_id
        .expect("the settled charge pays an invoice");
    assert_eq!(
        env.get_detailed_invoice(invoice_id)
            .await
            .invoice
            .amount_due,
        0
    );

    let session = env
        .store()
        .get_checkout_session(TENANT_ID, session.id)
        .await
        .expect("session");
    assert_eq!(session.status, CheckoutSessionStatus::Completed);
    assert!(session.subscription_id.is_some());
}

/// A decline is recorded on the attempt committed before it was sent, and the session reopens
/// for another one.
#[rstest]
#[tokio::test]
async fn test_declined_checkout_charge_is_recorded_on_its_attempt(#[future] test_env: TestEnv) {
    use meteroid_store::domain::checkout_sessions::CheckoutSessionStatus;

    let env = test_env.await;
    env.seed_payments().await;
    env.set_mock_charge_behavior(MockChargeBehavior::Failed)
        .await;

    let session = env
        .store()
        .create_checkout_session(self_serve_session(CUST_UBER_ID))
        .await
        .expect("create checkout session");
    let complete = || {
        env.services().complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3500,
            "EUR".to_string(),
            None,
        )
    };

    let err = complete().await.expect_err("a declined charge errors");
    assert!(matches!(
        err.current_context(),
        meteroid_store::errors::StoreError::PaymentError(_)
    ));
    let rows = session_transactions(&env, session.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].status,
        diesel_models::enums::PaymentStatusEnum::Failed
    );
    assert!(rows[0].provider_transaction_id.is_some());
    let reopened = env
        .store()
        .get_checkout_session(TENANT_ID, session.id)
        .await
        .expect("session");
    assert_eq!(reopened.status, CheckoutSessionStatus::Created);

    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    let CheckoutCompletionResult::Completed { .. } =
        complete().await.expect("a new attempt after the decline")
    else {
        panic!("a captured charge completes the checkout")
    };
    let mut statuses: Vec<_> = session_transactions(&env, session.id)
        .await
        .into_iter()
        .map(|row| row.status)
        .collect();
    statuses.sort_by_key(|status| format!("{status:?}"));
    assert_eq!(
        statuses,
        vec![
            diesel_models::enums::PaymentStatusEnum::Failed,
            diesel_models::enums::PaymentStatusEnum::Settled
        ]
    );
}

/// An activation checkout's charge with no answer back is fenced the same way, and its re-send
/// activates the subscription with a paid invoice.
#[rstest]
#[tokio::test]
async fn test_activation_charge_without_an_answer_activates_once_resent(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;
    env.set_mock_charge_behavior(MockChargeBehavior::Unknown)
        .await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
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
        .expect("activation checkout session");
    let complete = || {
        env.services().complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3500,
            "EUR".to_string(),
            None,
        )
    };

    let CheckoutCompletionResult::AwaitingPayment { transaction, .. } =
        complete().await.expect("an unanswered charge is in flight")
    else {
        panic!("an unanswered charge leaves the activation awaiting its payment")
    };
    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    let CheckoutCompletionResult::AwaitingPayment {
        transaction: again, ..
    } = complete().await.expect("re-completion is fenced")
    else {
        panic!("re-completion must wait for the unanswered charge")
    };
    assert_eq!(again.id, transaction.id);
    assert!(env.get_subscription(sub_id).await.activated_at.is_none());

    env.services()
        .resume_unsent_charge(transaction.id, TENANT_ID)
        .await
        .expect("resume the unsent charge");
    env.run_outbox_and_orchestration().await;

    let rows = session_transactions(&env, session.id).await;
    assert_eq!(rows.len(), 1, "the re-send is the same attempt");
    assert_eq!(
        rows[0].status,
        diesel_models::enums::PaymentStatusEnum::Settled
    );
    assert!(env.get_subscription(sub_id).await.activated_at.is_some());
    let invoice_id = rows[0]
        .invoice_id
        .expect("the settled charge pays an invoice");
    assert_eq!(
        env.get_detailed_invoice(invoice_id)
            .await
            .invoice
            .amount_due,
        0
    );
}

/// A capture settling for a checkout another charge already completed pays no invoice: it is kept
/// as the customer's credit, never lost.
#[rstest]
#[tokio::test]
async fn test_capture_for_a_completed_checkout_is_kept_as_credit(#[future] test_env: TestEnv) {
    use common_domain::ids::BaseId;

    let env = test_env.await;
    env.seed_payments().await;
    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;

    let session = env
        .store()
        .create_checkout_session(self_serve_session(CUST_UBER_ID))
        .await
        .expect("create checkout session");
    let CheckoutCompletionResult::Completed { .. } = env
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
        .expect("completion")
    else {
        panic!("a captured charge completes the checkout")
    };

    // A second charge of the session, committed and captured, that no run recorded.
    let orphan = common_domain::ids::PaymentTransactionId::new();
    {
        let mut conn = env.conn().await;
        diesel_models::payments::PaymentTransactionRowNew {
            id: orphan,
            tenant_id: TENANT_ID,
            invoice_id: None,
            provider_transaction_id: None,
            amount: 3500,
            currency: "EUR".to_string(),
            payment_method_id: Some(CUST_UBER_PAYMENT_METHOD_ID),
            status: diesel_models::enums::PaymentStatusEnum::Pending,
            payment_type: diesel_models::enums::PaymentTypeEnum::Payment,
            error_type: None,
            processed_at: None,
            checkout_session_id: Some(session.id),
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: Some(CUST_UBER_ID),
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            reversal_reason: None,
        }
        .insert(&mut conn)
        .await
        .expect("orphan charge");
    }
    env.deliver_mock_webhook(
        format!(
            r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"pi_{tx}","amount":3500,"currency":"EUR"}}"#,
            tx = orphan.as_base62(),
        )
        .as_bytes(),
    )
    .await;

    let kept: Vec<_> = super::credit_plan_change::grant_rows(&env, CUST_UBER_ID)
        .await
        .into_iter()
        .filter(|g| g.source_kind == "OVERPAYMENT" && g.source_id == orphan.as_uuid())
        .collect();
    assert_eq!(kept.len(), 1, "the capture is kept as credit");
    assert_eq!(kept[0].amount, 3500);
}

/// A charge the provider refused before creating anything is undone: no attempt is left and the
/// session reopens.
#[rstest]
#[tokio::test]
async fn test_refused_checkout_charge_is_undone(#[future] test_env: TestEnv) {
    use meteroid_store::domain::checkout_sessions::CheckoutSessionStatus;

    let env = test_env.await;
    env.seed_payments().await;
    env.set_mock_charge_behavior(MockChargeBehavior::RejectedCredentials)
        .await;

    let session = env
        .store()
        .create_checkout_session(self_serve_session(CUST_UBER_ID))
        .await
        .expect("create checkout session");
    let err = env
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
        .expect_err("a refused charge errors");
    assert!(matches!(
        err.current_context(),
        meteroid_store::errors::StoreError::PaymentProviderError
    ));

    assert!(session_transactions(&env, session.id).await.is_empty());
    let session = env
        .store()
        .get_checkout_session(TENANT_ID, session.id)
        .await
        .expect("session");
    assert_eq!(session.status, CheckoutSessionStatus::Created);
}

/// A Notion subscription with 5 seats at 10.00, from Jan 1, 2024.
async fn seats_subscription(env: &TestEnv) -> common_domain::ids::SubscriptionId {
    seats_billed(
        env,
        meteroid_store::domain::enums::BillingPeriodEnum::Monthly,
    )
    .await
}

/// [`seats_subscription`], at most `max` seats.
async fn seats_up_to(env: &TestEnv, max: u32) -> common_domain::ids::SubscriptionId {
    seats_with(
        env,
        meteroid_store::domain::enums::BillingPeriodEnum::Monthly,
        Some(max),
    )
    .await
}

/// [`seats_subscription`], its seats billed every `cadence`.
async fn seats_billed(
    env: &TestEnv,
    cadence: meteroid_store::domain::enums::BillingPeriodEnum,
) -> common_domain::ids::SubscriptionId {
    seats_with(env, cadence, None).await
}

async fn seats_with(
    env: &TestEnv,
    cadence: meteroid_store::domain::enums::BillingPeriodEnum,
    max_slots: Option<u32>,
) -> common_domain::ids::SubscriptionId {
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::{
        ComponentOverride, ComponentParameterization, ComponentParameters,
        CreateSubscriptionComponents,
    };

    let mut request = subscription()
        .plan_version(PLAN_VERSION_NOTION_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .request();
    request.price_components = Some(CreateSubscriptionComponents {
        parameterized_components: vec![ComponentParameterization {
            component_id: COMP_NOTION_SEATS_ID,
            parameters: ComponentParameters {
                initial_slot_count: Some(5),
                billing_period: None,
                committed_capacity: None,
            },
        }],
        overridden_components: vec![ComponentOverride {
            component_id: COMP_NOTION_SEATS_ID,
            name: "Seats".to_string(),
            price_entry: PriceEntry::New(PriceInput {
                cadence,
                currency: "EUR".to_string(),
                pricing: Pricing::Slot {
                    model: meteroid_store::domain::price_components::RatingModel::PerUnit {
                        rate: rust_decimal::Decimal::new(1000, 2),
                    },
                    min_slots: None,
                    max_slots,
                },
                credit_unit: None,
            }),
        }],
        extra_components: vec![],
        remove_components: vec![],
    });
    env.services()
        .insert_subscription(common_domain::actor::Actor::System, request, TENANT_ID)
        .await
        .expect("seats subscription")
        .id
}

async fn active_seats(env: &TestEnv, sub: common_domain::ids::SubscriptionId) -> u32 {
    use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;
    env.store()
        .get_active_slots_value(TENANT_ID, sub, QUANTITY_SEATS_ID, None)
        .await
        .expect("active seats")
}

async fn seat_purchase(
    env: &TestEnv,
    sub: common_domain::ids::SubscriptionId,
) -> meteroid_store::StoreResult<(meteroid_store::domain::PaymentTransaction, i32)> {
    env.services()
        .complete_slot_upgrade_checkout(
            TENANT_ID,
            sub,
            COMP_NOTION_SEATS_ID,
            2,
            CUST_UBER_PAYMENT_METHOD_ID,
            Some(
                NaiveDate::from_ymd_opt(2024, 1, 16)
                    .unwrap()
                    .and_hms_opt(12, 0, 0)
                    .unwrap(),
            ),
        )
        .await
        .map(|(transaction, seats, _)| (transaction, seats))
}

/// A seat purchase whose charge got no answer is committed with its invoice: the re-send under
/// its key settles it and the seats activate, charged once.
#[rstest]
#[tokio::test]
async fn test_seat_purchase_without_an_answer_activates_once_resent(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let sub = seats_subscription(&env).await;

    env.set_mock_charge_behavior(MockChargeBehavior::Unknown)
        .await;
    let (transaction, seats) = seat_purchase(&env, sub).await.expect("in flight");
    assert_eq!(transaction.status, PaymentStatusEnum::Pending);
    assert!(transaction.provider_transaction_id.is_none());
    assert_eq!((seats, active_seats(&env, sub).await), (5, 5));
    let invoice_id = transaction.invoice_id.expect("the purchase's invoice");

    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    env.services()
        .resume_unsent_charge(transaction.id, TENANT_ID)
        .await
        .expect("resume the unsent charge");
    env.run_outbox_and_orchestration().await;

    assert_eq!(active_seats(&env, sub).await, 7);
    let invoice = env.get_detailed_invoice(invoice_id).await;
    assert_eq!(invoice.invoice.amount_due, 0);
    assert_eq!(invoice.transactions.len(), 1, "charged once");
}

/// A declined seat purchase leaves nothing billed: the draft is closed, no seat is pending.
#[rstest]
#[tokio::test]
async fn test_declined_seat_purchase_leaves_nothing_billed(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let sub = seats_subscription(&env).await;

    env.set_mock_charge_behavior(MockChargeBehavior::Failed)
        .await;
    let err = seat_purchase(&env, sub).await.expect_err("declined");
    assert!(matches!(
        err.current_context(),
        meteroid_store::errors::StoreError::PaymentError(_)
    ));
    {
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::{invoice, slot_transaction};
        let mut conn = env.conn().await;
        let statuses: Vec<diesel_models::enums::InvoiceStatusEnum> = invoice::table
            .filter(invoice::subscription_id.eq(sub))
            .filter(invoice::status.eq(diesel_models::enums::InvoiceStatusEnum::Closed))
            .select(invoice::status)
            .load(&mut conn)
            .await
            .expect("invoices");
        assert_eq!(
            statuses,
            vec![diesel_models::enums::InvoiceStatusEnum::Closed]
        );
        let pending: i64 = slot_transaction::table
            .filter(slot_transaction::subscription_id.eq(sub))
            .filter(
                slot_transaction::status
                    .eq(diesel_models::enums::SlotTransactionStatusEnum::Pending),
            )
            .count()
            .get_result(&mut conn)
            .await
            .expect("pending seats");
        assert_eq!(pending, 0);
    }

    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    let (transaction, seats) = seat_purchase(&env, sub).await.expect("a new purchase");
    assert_eq!(transaction.status, PaymentStatusEnum::Settled);
    assert_eq!(
        (seats, active_seats(&env, sub).await),
        (7, 7),
        "only the paid seats"
    );
}

/// Seats billed quarterly on a monthly subscription (R-084): two bought on Jan 16 are prorated
/// over the quarter, 2 × 10.00 × 76/91 = 16.70, not over the month.
#[rstest]
#[tokio::test]
async fn test_seats_billed_quarterly_are_prorated_over_their_quarter(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let sub = seats_billed(
        &env,
        meteroid_store::domain::enums::BillingPeriodEnum::Quarterly,
    )
    .await;

    let (transaction, seats) = seat_purchase(&env, sub).await.expect("captured");
    assert_eq!(seats, 7);
    let invoice = env
        .get_detailed_invoice(transaction.invoice_id.expect("the purchase's invoice"))
        .await;
    let line = invoice
        .invoice
        .line_items
        .iter()
        .find(|line| line.amount_subtotal != 0)
        .expect("the seats line");
    assert_eq!(line.amount_subtotal, 1_670);
    assert_eq!(line.end_date, NaiveDate::from_ymd_opt(2024, 4, 1).unwrap());
}

/// Seats still paying count against the maximum: with 5 active, 2 pending and at most 8, two more
/// are refused instead of both purchases passing.
#[rstest]
#[tokio::test]
async fn test_pending_seats_count_against_the_maximum(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let sub = seats_up_to(&env, 8).await;

    env.set_mock_charge_behavior(MockChargeBehavior::Unknown)
        .await;
    seat_purchase(&env, sub).await.expect("in flight");
    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    let err = seat_purchase(&env, sub)
        .await
        .expect_err("over the maximum");
    assert!(format!("{err:?}").contains("maximum"), "{err:?}");
}

// =============================================================================
// (e) Credit and a settled payment cover the invoice: a second charge is refused
// =============================================================================

/// 3500 billed, 1750 paid by credit, 1750 charged and settled while its settlement event is still
/// queued: the account leaves nothing due, so a second charge is refused. Measured against the
/// total (1750 + 1750 ≤ 3500), it was taken.
/// Rules: R-127, R-125
#[rstest]
#[tokio::test]
async fn test_second_charge_refused_when_credit_and_a_settled_payment_cover_the_invoice(
    #[future] test_env: TestEnv,
) {
    use crate::helpers::credit::assert_ledger_balances;
    use crate::helpers::invariants::assert_invoice_accounts;

    let env = test_env.await;
    env.fund_invoice_credit(CUST_UBER_ID, 1_750).await;
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;
    let invoice = env.get_detailed_invoice(invoice_id).await.invoice;
    assert_eq!(
        (invoice.total, invoice.applied_credits, invoice.amount_due),
        (3_500, 1_750, 1_750)
    );

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
        .expect("first charge accepted");
    assert_eq!(tx.status, PaymentStatusEnum::Settled);
    assert_eq!(tx.amount, 1_750);

    let err = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect_err("credit and the settled charge cover the invoice");
    assert!(
        format!("{err:?}").contains("sufficient payments"),
        "unexpected error: {err:?}"
    );
    let payments = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list payment transactions");
    assert_eq!(payments.len(), 1, "no second charge was recorded");
    assert_eq!(payments[0].transaction.amount, 1_750);

    env.run_outbox_and_orchestration().await;
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid()
        .has_total(3_500)
        .has_applied_credits(1_750)
        .has_amount_due(0);
    assert_eq!(env.invoice_credit_balance_minor(CUST_UBER_ID).await, 0);
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    assert_invoice_accounts(env.store(), TENANT_ID).await;
}
