//! Checkout completion tests.
//!
//! Tests for completing checkout flow:
//! - PendingActivation → Active
//! - During free trial (saves payment method)
//! - After trial expires (reactivation)
//! - Payment failures

use chrono::NaiveDate;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env};
use diesel_models::enums::CycleActionEnum;

// =============================================================================
// CHECKOUT FROM PENDING ACTIVATION
// =============================================================================

/// Complete checkout for PendingActivation → Active with invoice
#[rstest]
#[tokio::test]
async fn test_checkout_from_pending_activates_subscription(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    // Verify pending state
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_pending_activation()
        .has_pending_checkout(true);

    // Complete checkout
    let mut conn = env.conn().await;
    let (transaction, is_pending) = env
        .services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3500, // $35.00
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed");

    assert!(transaction.is_some(), "Should have payment transaction");
    assert!(!is_pending, "Payment should not be pending");

    // Verify Active state
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(true);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).has_total(3500);
}

// =============================================================================
// CHECKOUT DURING FREE TRIAL
// =============================================================================

/// Checkout during free trial: saves payment method, no charge, trial continues
#[rstest]
#[tokio::test]
async fn test_checkout_during_free_trial_saves_payment_method(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_checkout()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;

    // Verify trial state
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active().has_pending_checkout(true);

    // Complete checkout during trial (no charge)
    let mut conn = env.conn().await;
    let (transaction, _) = env
        .services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0, // No charge for free trial
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed");

    assert!(transaction.is_none(), "No payment during free trial");

    // Verify trial continues with payment method saved
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active() // Still in trial
        .has_pending_checkout(false) // Checkout completed
        .has_payment_method(true); // PM saved

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty(); // No invoice yet
}

/// Free trial with checkout completed: trial ends → Active with auto-charge
#[rstest]
#[tokio::test]
async fn test_free_trial_with_checkout_auto_charges_at_trial_end(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_checkout()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;

    // Complete checkout during trial
    let mut conn = env.conn().await;
    env.services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed");

    // Verify checkout completed
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(false)
        .has_payment_method(true);

    // Process trial end
    env.process_cycles().await;

    // Verify Active with invoice
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_payment_method(true);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).has_total(4900); // $49.00
}

// =============================================================================
// CHECKOUT AFTER TRIAL EXPIRED
// =============================================================================

/// Checkout after TrialExpired reactivates subscription
#[rstest]
#[tokio::test]
async fn test_checkout_after_trial_expired_reactivates(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_checkout()
        .trial_days(14)
        .no_auto_charge() // Ensure trial expires
        .create(env.services())
        .await;

    // Let trial expire
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_expired().has_pending_checkout(true);

    // Complete checkout after expiry
    let mut conn = env.conn().await;
    let (transaction, is_pending) = env
        .services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            4900, // Now we charge
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed");

    assert!(transaction.is_some(), "Should have payment");
    assert!(!is_pending);

    // Verify reactivated
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(true);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
}

// =============================================================================
// PAID TRIAL CHECKOUT
// =============================================================================

/// OnCheckout + Paid Trial: checkout with full payment → TrialActive → Active
///
/// With paid trials, billing is decoupled from trial status:
/// - Checkout creates invoice 1 (trial active, bills immediately)
/// - First process_cycles fires both trial end AND month-1 renewal → invoice 2
///
/// The trial end event (day 7) does NOT create an invoice.
/// The renewal (day 31) creates invoice 2.
#[rstest]
#[tokio::test]
async fn test_oncheckout_paid_trial_full_flow(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID) // $99/month
        .start_date(start_date)
        .on_checkout()
        .trial_days(7)
        .auto_charge()
        .create(env.services())
        .await;

    // === Phase 1: PendingActivation ===
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Phase 1: PendingActivation")
        .is_pending_activation()
        .has_pending_checkout(true);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // === Phase 2: Complete checkout with full payment ===
    let mut conn = env.conn().await;
    let (transaction, _) = env
        .services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            9900, // Full $99 - NOT prorated
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed");

    assert!(transaction.is_some(), "Should have payment for paid trial");

    // After checkout → TrialActive with 1 invoice
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Phase 2: After checkout")
        .is_trial_active()
        .has_next_action(Some(CycleActionEnum::RenewSubscription)) // Paid trial = normal billing
        .has_pending_checkout(false)
        .has_payment_method(true)
        .has_trial_duration(Some(7))
        .has_cycle_index(0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("Invoice 1: at checkout")
        .has_total(9900)
        .has_invoice_date(start_date)
        .check_prorated(false);

    // === Phase 3: process_cycles - trial ends AND renewal fires ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Phase 3: After process_cycles")
        .is_active()
        .has_cycle_index(1); // Cycle advanced due to renewal

    // 2 invoices: checkout + first renewal
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);

    // Invoice 2 is at Feb 1 (billing cycle end), NOT at Jan 8 (trial end)
    invoices
        .assert()
        .invoice_at(1)
        .with_context("Invoice 2: first renewal, not trial end")
        .has_total(9900)
        .has_invoice_date(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap());

    // Verify: no invoice on Jan 8 (trial end date)
    let trial_end_date = NaiveDate::from_ymd_opt(2024, 1, 8).unwrap();
    for invoice in invoices.iter() {
        assert_ne!(
            invoice.invoice_date, trial_end_date,
            "No invoice should be dated at trial end"
        );
    }
}

// =============================================================================
// PAYMENT FAILURE TESTS
// =============================================================================

/// Failed payment keeps subscription in pending state
#[rstest]
#[tokio::test]
async fn test_checkout_failed_payment_stays_pending(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(true).await; // Payments will FAIL
    env.seed_customer_payment_methods().await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_pending_activation();

    // Attempt checkout - should fail
    let mut conn = env.conn().await;
    let result = env
        .services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3500,
            "EUR".to_string(),
            None,
        )
        .await;

    assert!(result.is_err(), "Checkout should fail");
    env.run_outbox_and_orchestration().await;
    env.process_cycles().await;

    // Subscription should still be pending
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_pending_activation()
        .has_pending_checkout(true)
        .has_payment_method(true);
}

// =============================================================================
// CHECKOUT AFTER FREE TRIAL EXPIRY
// =============================================================================

/// OnCheckout + Free Trial: checkout after trial expires reactivates with payment
/// Scenario: TrialActive → (no checkout, trial expires) → TrialExpired → (checkout) → Active
#[rstest]
#[tokio::test]
async fn test_oncheckout_free_trial_checkout_after_expiry(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month with 14-day free trial
        .on_checkout()
        .trial_days(14)
        .no_auto_charge() // No auto-charge to ensure expiry
        .create(env.services())
        .await;

    // Process trial end to get to TrialExpired
    env.process_cycles().await;
    env.run_outbox_and_orchestration().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_expired().has_pending_checkout(true);

    // Complete checkout after expiry
    let mut conn = env.conn().await;
    let (transaction, is_pending) = env
        .services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            4900, // Now we charge
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed");

    assert!(transaction.is_some(), "Should have payment transaction");
    assert!(!is_pending, "Payment should not be pending");

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(true)
        .has_trial_duration(Some(14));

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_total(4900)
        .check_prorated(false);

    env.run_outbox_and_orchestration().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_paid()
        .has_total(4900)
        .check_prorated(false);
}

// =============================================================================
// PAYMENT FAILURE - DETAILED ASSERTIONS
// =============================================================================

/// OnCheckout + payment fails at checkout: subscription stays PendingActivation
/// Verifies that failed checkout doesn't activate subscription and no paid invoices exist
#[rstest]
#[tokio::test]
async fn test_oncheckout_payment_failure_at_checkout(#[future] test_env: TestEnv) {
    use meteroid_store::domain::enums::InvoicePaymentStatus;

    let env = test_env.await;
    env.seed_mock_payment_provider(true).await; // Payments will FAIL
    env.seed_customer_payment_methods().await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_pending_activation();

    // Attempt checkout - should fail
    let mut conn = env.conn().await;
    let result = env
        .services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3500,
            "EUR".to_string(),
            None,
        )
        .await;

    assert!(result.is_err(), "Checkout should fail with payment failure");

    env.run_outbox_and_orchestration().await;

    // Subscription should still be PendingActivation
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_pending_activation()
        .has_pending_checkout(true)
        .has_payment_method(true)
        .has_trial_duration(None);

    // No completed invoice - may have draft from failed attempt
    let invoices = env.get_invoices(sub_id).await;
    for invoice in invoices.iter() {
        assert_ne!(
            invoice.payment_status,
            InvoicePaymentStatus::Paid,
            "No invoice should be paid after failed checkout"
        );
    }
}
