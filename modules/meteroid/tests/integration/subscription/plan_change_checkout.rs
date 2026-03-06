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

// =============================================================================
// DOWNGRADE → UPGRADE ROUNDTRIP (CREDITS APPLIED CORRECTLY)
// =============================================================================

/// Downgrade then upgrade back: credits from downgrade should be applied to upgrade charge.
///
/// Start on Pro ($124/mo), period [Jan 1, Feb 1] = 31 days.
/// Step 1: Downgrade to Starter ($39/mo) via checkout on Jan 16.
///   factor = 16/31
///   Credit Platform: -(9900 × 16/31).round() = -5110
///   Credit Seats:    -(2500 × 16/31).round() = -1290
///   Charge Platform:  (2900 × 16/31).round() = 1497
///   Charge Seats:     (1000 × 16/31).round() = 516
///   Net: -4387 (customer gets credit)
///
/// Step 2: Upgrade back to Pro ($124/mo) via checkout on Jan 16.
///   Credit Platform: -(2900 × 16/31).round() = -1497
///   Credit Seats:    -(1000 × 16/31).round() = -516
///   Charge Platform:  (9900 × 16/31).round() = 5110
///   Charge Seats:     (2500 × 16/31).round() = 1290
///   Net: +4387 (upgrade charge)
///   But customer has 4387 balance → applied_credits = 4387, amount_due = 0
///   No payment transaction needed.
#[rstest]
#[tokio::test]
async fn test_plan_change_checkout_downgrade_then_upgrade_credits_applied(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    // Start on Pro plan ($124/mo) via checkout
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PRO_ID)
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
            12400,
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

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).has_total(12400);

    // Step 1: Downgrade to Starter ($39/mo) on Jan 16
    let session_down = env
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
        .expect("create downgrade session failed");

    // Net = -4387 (downgrade), no charge
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session_down.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Downgrade checkout should succeed");

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

    // Verify: now on Starter, customer has credit
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_STARTER_ID);
    sub.assert().is_active();

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("downgrade adjustment")
        .has_total(-4387)
        .has_amount_due(0)
        .has_applied_credits(0)
        .is_finalized_paid();

    let customer = env.get_customer(sub.customer_id).await;
    assert_eq!(
        customer.balance_value_cents, 4387,
        "customer should have 4387 cents credit from downgrade"
    );

    // Step 2: Upgrade back to Pro ($124/mo) on Jan 16
    // Preview first to verify credits are reflected
    let preview = env
        .services()
        .compute_plan_change_checkout_invoice(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, change_date)
        .await
        .expect("preview should succeed");

    assert_eq!(preview.total, 4387, "Preview total should be net proration");
    assert_eq!(
        preview.applied_credits, 4387,
        "Preview should show full credits applied"
    );
    assert_eq!(
        preview.amount_due, 0,
        "Preview amount_due should be 0 (credits cover upgrade)"
    );

    let session_up = env
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
        .expect("create upgrade session failed");

    // amount_due = 0 (credits cover the full upgrade)
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session_up.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Upgrade checkout should succeed (credits cover charge)");

    match result {
        CheckoutCompletionResult::Completed {
            subscription_id,
            transaction,
        } => {
            assert_eq!(subscription_id, sub_id);
            assert!(
                transaction.is_none(),
                "No payment transaction when credits cover full amount"
            );
        }
        CheckoutCompletionResult::AwaitingPayment { .. } => {
            panic!("Should not require payment when credits cover charge")
        }
    };

    // Verify: back on Pro, credits consumed
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_PRO_ID);
    sub.assert().is_active().has_pending_checkout(false);

    // 3 invoices: initial Pro + downgrade adjustment + upgrade adjustment
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    invoices
        .assert()
        .invoice_at(2)
        .with_context("upgrade adjustment with credits")
        .has_total(4387)
        .has_applied_credits(4387)
        .has_amount_due(0)
        .is_finalized_paid();

    // Customer balance should be 0 (credits fully consumed)
    let customer = env.get_customer(sub.customer_id).await;
    assert_eq!(
        customer.balance_value_cents, 0,
        "customer balance should be 0 after credits consumed by upgrade"
    );

    // Verify renewal works at Pro rate with no credits
    env.process_cycles().await;
    // Run full billing pipeline: finalize grace-period invoices, generate PDFs, process payments
    env.run_outbox_and_orchestration().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(1);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(4);
    invoices
        .assert()
        .invoice_at(3)
        .with_context("renewal at Pro rate, no credits")
        .has_total(12400)
        .has_applied_credits(0);

    // Verify: payment transaction is for full amount (no credits)
    let renewal_invoice = &invoices[3];
    let detailed = env.get_detailed_invoice(renewal_invoice.id).await;
    assert_eq!(
        detailed.transactions.len(),
        1,
        "renewal should have exactly 1 payment transaction"
    );
    assert_eq!(
        detailed.transactions[0].amount, 12400,
        "payment should be for full amount (no credits to deduct)"
    );

    // Customer balance still 0
    let customer = env.get_customer(sub.customer_id).await;
    assert_eq!(customer.balance_value_cents, 0);
}

/// Upgrade with partial credits: downgrade to cheaper plan, then upgrade to a middle plan.
/// Credits partially cover the upgrade, requiring a smaller payment.
///
/// Start on Pro ($124/mo), period [Jan 1, Feb 1] = 31 days.
/// Step 1: Downgrade to Starter ($39/mo) via checkout on Jan 21.
///   factor = 11/31
///   Credit Platform: -(9900 × 11/31).round() = -3513
///   Credit Seats:    -(2500 × 11/31).round() = -887
///   Charge Platform:  (2900 × 11/31).round() = 1029
///   Charge Seats:     (1000 × 11/31).round() = 355
///   Net: -3016 (customer gets 3016 credit)
///
/// Step 2: Upgrade to PaidFreeTrial ($49/mo) via checkout on Jan 21.
///   Credit Platform: -(2900 × 11/31).round() = -1029
///   Credit Seats:    -(1000 × 11/31).round() = -355
///   Charge Rate:      (4900 × 11/31).round() = 1739
///   Net: 355 (small upgrade charge)
///   Customer has 3016 balance → applied_credits = 355, amount_due = 0
#[rstest]
#[tokio::test]
async fn test_plan_change_checkout_partial_credits_on_upgrade(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 21).unwrap();

    // Start on Pro ($124/mo) via checkout
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PRO_ID)
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
            12400,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Initial checkout should succeed");
    drop(conn);

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    // Step 1: Downgrade to Starter ($39/mo) on Jan 21
    let session_down = env
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
        .expect("create downgrade session failed");

    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session_down.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Downgrade checkout should succeed");

    match &result {
        CheckoutCompletionResult::Completed { transaction, .. } => {
            assert!(transaction.is_none(), "No payment for downgrade");
        }
        _ => panic!("Expected Completed"),
    };

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_STARTER_ID);

    let customer = env.get_customer(sub.customer_id).await;
    assert_eq!(
        customer.balance_value_cents, 3016,
        "customer should have 3016 cents credit from downgrade (Pro→Starter, 11/31 factor)"
    );

    // Step 2: Upgrade to PaidFreeTrial ($49/mo) on Jan 21
    // Net = 355, but customer has 3016 balance → credits cover it fully
    let preview = env
        .services()
        .compute_plan_change_checkout_invoice(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PAID_FREE_TRIAL_ID,
            change_date,
        )
        .await
        .expect("preview should succeed");

    assert_eq!(preview.total, 355, "Preview total = net proration");
    assert_eq!(
        preview.applied_credits, 355,
        "Credits applied = min(total, balance)"
    );
    assert_eq!(preview.amount_due, 0, "Credits cover the full charge");

    let session_up = env
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
        .expect("create upgrade session failed");

    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session_up.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Upgrade checkout should succeed");

    match &result {
        CheckoutCompletionResult::Completed { transaction, .. } => {
            assert!(
                transaction.is_none(),
                "No payment when credits cover full amount"
            );
        }
        _ => panic!("Expected Completed"),
    };

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_PAID_FREE_TRIAL_ID);
    sub.assert().is_active();

    // Customer balance: 3016 - 355 = 2661
    let customer = env.get_customer(sub.customer_id).await;
    assert_eq!(
        customer.balance_value_cents, 2661,
        "customer balance should be 3016 - 355 = 2661 after partial credit use"
    );

    // 3 invoices: initial + downgrade adj + upgrade adj
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    invoices
        .assert()
        .invoice_at(2)
        .with_context("upgrade adj with partial credits")
        .has_total(355)
        .has_applied_credits(355)
        .has_amount_due(0)
        .is_finalized_paid();

    // Step 3: Renewal — verify credits are applied to the renewal invoice
    // PaidFreeTrial = €49/mo = 4900 cents
    // Customer has 2661 balance → applied_credits = 2661, amount_due = 4900 - 2661 = 2239
    env.process_cycles().await;
    // Run full billing pipeline: finalize grace-period invoices, generate PDFs, process payments
    env.run_outbox_and_orchestration().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(1);

    // 4 invoices: initial + downgrade adj + upgrade adj + renewal
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(4);
    invoices
        .assert()
        .invoice_at(3)
        .with_context("renewal with remaining credits")
        .has_total(4900)
        .has_applied_credits(2661);

    // Customer balance fully consumed by renewal
    let customer = env.get_customer(sub.customer_id).await;
    assert_eq!(
        customer.balance_value_cents, 0,
        "customer balance should be 0 after renewal consumed remaining credits"
    );

    // Verify the renewal invoice has a payment transaction for the reduced amount (not full total)
    let renewal_invoice = &invoices[3];
    let detailed = env.get_detailed_invoice(renewal_invoice.id).await;
    assert_eq!(
        detailed.transactions.len(),
        1,
        "renewal should have exactly 1 payment transaction"
    );
    assert_eq!(
        detailed.transactions[0].amount, 2239,
        "payment transaction should be for amount_due (2239), not total (4900)"
    );
}
