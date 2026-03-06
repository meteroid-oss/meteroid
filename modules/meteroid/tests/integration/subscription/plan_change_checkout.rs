//! Plan change checkout integration tests.
//!
//! Tests for plan change via the checkout session flow (with payment):
//! - Free trial → no-trial plan (charge first period, no adjustment)
//! - Paid trial → no-trial plan (adjustment invoice, same as non-trial)
//! - No-trial → trial plan (trial only at signup, normal proration)
//! - Payment failure scenarios

use chrono::NaiveDate;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env};
use meteroid_store::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutType};

// =============================================================================
// FREE TRIAL → NO-TRIAL PLAN (CHECKOUT WITH PAYMENT)
// =============================================================================

/// Upgrade from free trial plan to a no-trial plan via checkout.
///
/// Customer on PLAN_VERSION_PAID_FREE_TRIAL_ID ($49/mo, 14d free trial) in TrialActive.
/// Upgrades to PLAN_VERSION_STARTER_ID ($39/mo, no trial) on day 8.
///
/// Free trial: no adjustment (nothing billed yet).
/// Checkout charges first full period of new plan (Starter: €29 + €10 = €39).
/// Subscription transitions TrialActive → Active on new plan.
#[rstest]
#[tokio::test]
async fn test_plan_change_checkout_from_free_trial_to_no_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 8).unwrap(); // Day 8 of 14-day trial

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/mo, 14d free trial
        .start_date(start_date)
        .on_start()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_period_start(start_date)
        .has_period_end(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            PLAN_VERSION_STARTER_ID, // $39/mo, no trial
            sub.customer_id,
            USER_ID,
            None,
            change_date,
        )
        .await
        .expect("create_plan_change_checkout_session failed");

    assert_eq!(session.checkout_type, CheckoutType::PlanChange);

    // Starter: Platform Fee €29 + Seats €10 = €39/mo = 3900 cents
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3900,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed");

    let subscription_id = match result {
        CheckoutCompletionResult::Completed {
            subscription_id, ..
        } => subscription_id,
        CheckoutCompletionResult::AwaitingPayment { .. } => {
            panic!("Payment should not be pending for mock provider")
        }
    };

    assert_eq!(subscription_id, sub_id);

    // Now Active on Starter plan, billing restarts from change_date
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_STARTER_ID);
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_period_start(change_date)
        .has_period_end(NaiveDate::from_ymd_opt(2024, 2, 8).unwrap())
        .has_resolved_payment_method(&env, true)
        .await;

    // 1 invoice at Starter rate for [Jan 8, Feb 8]
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("first invoice after free trial plan change")
        .has_total(3900)
        .has_invoice_date(change_date);

    // Verify renewal works
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(1)
        .has_period_start(NaiveDate::from_ymd_opt(2024, 2, 8).unwrap())
        .has_period_end(NaiveDate::from_ymd_opt(2024, 3, 8).unwrap());

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("renewal at Starter rate")
        .has_total(3900);
}

/// Payment failure during free trial → no-trial plan change checkout.
/// Plan change should NOT be applied, subscription stays in TrialActive.
#[rstest]
#[tokio::test]
async fn test_plan_change_checkout_from_free_trial_payment_fails(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(true).await; // Payments will FAIL
    env.seed_customer_payment_methods().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 8).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(start_date)
        .on_start()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active();

    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            PLAN_VERSION_STARTER_ID,
            sub.customer_id,
            USER_ID,
            None,
            change_date,
        )
        .await
        .expect("create session failed");

    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3900,
            "EUR".to_string(),
            None,
        )
        .await;

    assert!(result.is_err(), "Checkout should fail when payment fails");

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_PAID_FREE_TRIAL_ID);
    sub.assert().is_trial_active();

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

// =============================================================================
// PAID TRIAL → NO-TRIAL PLAN (CHECKOUT WITH PAYMENT)
// =============================================================================

/// Downgrade from paid trial ($99/mo) to no-trial plan ($39/mo) via checkout.
///
/// start Jan 1, paid trial 7d, period [Jan 1, Feb 1] = 31 days, change on Jan 8.
/// factor = 24/31 (24 days remaining)
///
/// Paid Trial: Rate 9900/mo
/// Starter: Platform Fee 2900 + Seats 1000 = 3900/mo
///
/// Credit Rate:      -(9900 × 24/31).round() = -7665
/// Charge Platform:   (2900 × 24/31).round() = 2245
/// Charge Seats:      (1000 × 24/31).round() = 774
/// Net: -7665 + 2245 + 774 = -4646 (credit/downgrade, no charge needed)
#[rstest]
#[tokio::test]
async fn test_plan_change_checkout_from_paid_trial_downgrade(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 8).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID) // $99/mo, 7d paid trial
        .start_date(start_date)
        .on_checkout()
        .trial_days(7)
        .auto_charge()
        .create(env.services())
        .await;

    // Complete initial checkout to enter TrialActive
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
        .expect("Initial checkout should succeed");
    drop(conn);

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_period_start(start_date)
        .has_period_end(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap());

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).has_total(9900);

    // Downgrade to Starter ($39) at Jan 8
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            PLAN_VERSION_STARTER_ID,
            sub.customer_id,
            USER_ID,
            None,
            change_date,
        )
        .await
        .expect("create session failed");

    // Net is negative (downgrade), no charge needed
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed for downgrade");

    match result {
        CheckoutCompletionResult::Completed {
            subscription_id,
            transaction,
        } => {
            assert_eq!(subscription_id, sub_id);
            assert!(transaction.is_none(), "No payment for downgrade");
        }
        CheckoutCompletionResult::AwaitingPayment { .. } => {
            panic!("Downgrade should not require payment")
        }
    };

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_STARTER_ID);
    sub.assert().is_active();

    // Initial paid trial invoice + adjustment invoice (negative)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("adjustment invoice for downgrade")
        .has_total(-4646)
        .check_prorated(true);

    // Verify renewal at Starter rate
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    invoices
        .assert()
        .invoice_at(2)
        .with_context("renewal at Starter rate")
        .has_total(3900);
}

/// Upgrade from paid trial ($99/mo, 1 Rate) to Pro ($124/mo, 2 components) via checkout.
///
/// start Jan 1, period [Jan 1, Feb 1] = 31 days, change on Jan 16.
/// factor = 16/31
///
/// Credit Rate:      -(9900 × 16/31).round() = -5110
/// Charge Platform:   (9900 × 16/31).round() = 5110
/// Charge Seats:      (2500 × 16/31).round() = 1290
/// Net: -5110 + 5110 + 1290 = 1290
#[rstest]
#[tokio::test]
async fn test_plan_change_checkout_from_paid_trial_upgrade(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID) // $99/mo, 7d paid trial
        .start_date(start_date)
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
        .expect("Initial checkout should succeed");
    drop(conn);

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active();

    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            PLAN_VERSION_PRO_ID, // $124/mo
            sub.customer_id,
            USER_ID,
            None,
            change_date,
        )
        .await
        .expect("create session failed");

    // Net = 1290 (upgrade)
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            1290,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed for upgrade");

    match result {
        CheckoutCompletionResult::Completed {
            subscription_id,
            transaction,
        } => {
            assert_eq!(subscription_id, sub_id);
            assert!(transaction.is_some(), "Should have payment for upgrade");
        }
        CheckoutCompletionResult::AwaitingPayment { .. } => {
            panic!("Payment should not be pending for mock provider")
        }
    };

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_PRO_ID);
    sub.assert().is_active();

    // Initial invoice + adjustment invoice
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("adjustment invoice for upgrade")
        .has_total(1290)
        .check_prorated(true);

    // Verify renewal at Pro rate
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    invoices
        .assert()
        .invoice_at(2)
        .with_context("renewal at Pro rate")
        .has_total(12400);
}

// =============================================================================
// NO-TRIAL → TRIAL PLAN (TRIAL ONLY AT SIGNUP)
// =============================================================================

/// Upgrade from no-trial plan to a plan with trial config.
/// Trial is NOT applied (trials only at initial subscription creation).
///
/// start Jan 1, period [Jan 1, Feb 1] = 31 days, change on Jan 16.
/// factor = 16/31
///
/// Starter: Platform Fee 2900 + Seats 1000 = 3900/mo
/// PaidFreeTrial: Rate 4900/mo
///
/// Credit Platform: -(2900 × 16/31).round() = -1497
/// Credit Seats:    -(1000 × 16/31).round() = -516
/// Charge Rate:      (4900 × 16/31).round() = 2529
/// Net: -1497 + -516 + 2529 = 516
#[rstest]
#[tokio::test]
async fn test_plan_change_checkout_to_trial_plan_no_trial_applied(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID) // $39/mo
        .start_date(start_date)
        .on_checkout()
        .no_trial()
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
            3900,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Initial checkout should succeed");
    drop(conn);

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_period_start(start_date)
        .has_period_end(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap());

    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            PLAN_VERSION_PAID_FREE_TRIAL_ID, // $49/mo with trial config
            sub.customer_id,
            USER_ID,
            None,
            change_date,
        )
        .await
        .expect("create session failed");

    // Net = 516 (upgrade)
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            516,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed");

    match result {
        CheckoutCompletionResult::Completed {
            subscription_id,
            transaction,
        } => {
            assert_eq!(subscription_id, sub_id);
            assert!(transaction.is_some(), "Should have payment for upgrade");
        }
        CheckoutCompletionResult::AwaitingPayment { .. } => {
            panic!("Payment should not be pending for mock provider")
        }
    };

    // Active (NOT TrialActive) on new plan
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_PAID_FREE_TRIAL_ID);
    sub.assert().is_active().has_pending_checkout(false);

    // Initial invoice + adjustment invoice
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("adjustment invoice for upgrade")
        .has_total(516)
        .check_prorated(true);

    // Verify renewal at new rate
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    invoices
        .assert()
        .invoice_at(2)
        .with_context("renewal at PaidFreeTrial rate")
        .has_total(4900);
}

/// Payment failure: no-trial → trial plan checkout fails, subscription unchanged.
#[rstest]
#[tokio::test]
async fn test_plan_change_checkout_to_trial_plan_payment_fails(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    // Complete initial checkout (succeeds with working payment)
    let mut conn = env.conn().await;
    env.services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3900,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Initial checkout should succeed");
    drop(conn);

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    // Switch payment provider to FAIL mode
    env.set_mock_payment_failure(true).await;

    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            PLAN_VERSION_PAID_FREE_TRIAL_ID,
            sub.customer_id,
            USER_ID,
            None,
            change_date,
        )
        .await
        .expect("create session failed");

    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            516,
            "EUR".to_string(),
            None,
        )
        .await;

    assert!(result.is_err(), "Checkout should fail when payment fails");

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_STARTER_ID);
    sub.assert().is_active();

    // Only the initial Starter invoice
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
}

// =============================================================================
// VALIDATION
// =============================================================================

/// Checkout fails if amount confirmation doesn't match expected charge.
#[rstest]
#[tokio::test]
async fn test_plan_change_checkout_amount_mismatch_rejected(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 8).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(start_date)
        .on_start()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active();

    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            PLAN_VERSION_STARTER_ID,
            sub.customer_id,
            USER_ID,
            None,
            change_date,
        )
        .await
        .expect("create session failed");

    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            9999, // Wrong amount (expected 3900)
            "EUR".to_string(),
            None,
        )
        .await;

    assert!(result.is_err(), "Checkout should fail with amount mismatch");

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_PAID_FREE_TRIAL_ID);
    sub.assert().is_trial_active();
}

/// Checkout fails if currency doesn't match.
#[rstest]
#[tokio::test]
async fn test_plan_change_checkout_currency_mismatch_rejected(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 8).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(start_date)
        .on_start()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;

    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            PLAN_VERSION_STARTER_ID,
            sub.customer_id,
            USER_ID,
            None,
            change_date,
        )
        .await
        .expect("create session failed");

    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3900,
            "USD".to_string(), // Wrong currency (expected EUR)
            None,
        )
        .await;

    assert!(
        result.is_err(),
        "Checkout should fail with currency mismatch"
    );
}

// =============================================================================
// ROUND-TRIP: compute_plan_change_checkout_invoice → complete_checkout
// Validates GetCheckout and ConfirmCheckout agree on amounts.
// =============================================================================

/// Round-trip: free trial → no-trial. Preview amount matches completion.
#[rstest]
#[tokio::test]
async fn test_plan_change_checkout_roundtrip_free_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 8).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/mo, 14d free trial
        .start_date(start_date)
        .on_start()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active();

    // Compute preview (same as GetCheckout)
    let preview = env
        .services()
        .compute_plan_change_checkout_invoice(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_STARTER_ID,
            change_date,
        )
        .await
        .expect("preview should succeed");

    assert_eq!(
        preview.total, 3900,
        "Preview total should be Starter first period"
    );

    // Use preview amount for checkout (round-trip)
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            PLAN_VERSION_STARTER_ID,
            sub.customer_id,
            USER_ID,
            None,
            change_date,
        )
        .await
        .expect("create session failed");

    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            preview.total as u64,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout with preview amount should succeed");

    match result {
        CheckoutCompletionResult::Completed {
            subscription_id, ..
        } => {
            assert_eq!(subscription_id, sub_id);
        }
        _ => panic!("Expected Completed"),
    };
}

/// Round-trip: paid trial upgrade. Preview amount matches completion.
#[rstest]
#[tokio::test]
async fn test_plan_change_checkout_roundtrip_paid_trial_upgrade(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID) // $99/mo, 7d paid trial
        .start_date(start_date)
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
        .expect("Initial checkout should succeed");
    drop(conn);

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active();

    // Compute preview
    let preview = env
        .services()
        .compute_plan_change_checkout_invoice(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID, // $124/mo
            change_date,
        )
        .await
        .expect("preview should succeed");

    assert_eq!(preview.total, 1290, "Preview total should be net proration");

    // Use preview amount for checkout
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            PLAN_VERSION_PRO_ID,
            sub.customer_id,
            USER_ID,
            None,
            change_date,
        )
        .await
        .expect("create session failed");

    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            preview.total as u64,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout with preview amount should succeed");

    match result {
        CheckoutCompletionResult::Completed {
            subscription_id, ..
        } => {
            assert_eq!(subscription_id, sub_id);
        }
        _ => panic!("Expected Completed"),
    };
}

/// Round-trip: paid trial downgrade (credit). Preview total is negative, charge 0.
#[rstest]
#[tokio::test]
async fn test_plan_change_checkout_roundtrip_downgrade(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 8).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID) // $99/mo
        .start_date(start_date)
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
        .expect("Initial checkout should succeed");
    drop(conn);

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active();

    // Compute preview for downgrade
    let preview = env
        .services()
        .compute_plan_change_checkout_invoice(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_STARTER_ID, // $39/mo (downgrade)
            change_date,
        )
        .await
        .expect("preview should succeed");

    // Net is negative (-4646), total should reflect the credit
    assert!(
        preview.total <= 0,
        "Preview total for downgrade should be <= 0, got {}",
        preview.total
    );

    // Downgrade: charge 0
    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            PLAN_VERSION_STARTER_ID,
            sub.customer_id,
            USER_ID,
            None,
            change_date,
        )
        .await
        .expect("create session failed");

    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed for downgrade");

    match result {
        CheckoutCompletionResult::Completed {
            subscription_id, ..
        } => {
            assert_eq!(subscription_id, sub_id);
        }
        _ => panic!("Expected Completed"),
    };
}
