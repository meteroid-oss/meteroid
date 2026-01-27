//! Subscription lifecycle and renewal tests.
//!
//! Tests for:
//! - Renewal cycles
//! - Period transitions
//! - Multi-cycle scenarios
//!
//! Ported from test_subscription_lifecycle.rs

use chrono::NaiveDate;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env};

// =============================================================================
// BASIC RENEWAL TESTS
// =============================================================================

/// OnStart: Active → renewal → Active with new invoice
#[rstest]
#[tokio::test]
async fn test_onstart_renewal_creates_new_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Initial state: cycle 0
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);

    // First renewal: cycle 1
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(1);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .is_finalized_unpaid()
        .has_total(3500);

    // Second renewal: cycle 2
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(2);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
}

/// Multiple renewals verify period dates advance correctly
#[rstest]
#[tokio::test]
async fn test_renewal_advances_period_dates(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Cycle 0
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.current_period_start, start_date);
    let period_end_0 = sub.current_period_end.expect("Should have period end");

    // Cycle 1
    env.process_cycles().await;
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(
        sub.current_period_start, period_end_0,
        "Period start should be previous period end"
    );
    let period_end_1 = sub.current_period_end.expect("Should have period end");
    assert!(period_end_1 > period_end_0, "Period end should advance");

    // Cycle 2
    env.process_cycles().await;
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.current_period_start, period_end_1);
    let period_end_2 = sub.current_period_end.expect("Should have period end");
    assert!(period_end_2 > period_end_1, "Period end should advance");
}

// =============================================================================
// RENEWAL AFTER FREE TRIAL
// =============================================================================

/// OnCheckout + Free Trial: checkout during trial → trial ends → 2 renewals
/// Total: 3 invoices at the end
#[rstest]
#[tokio::test]
async fn test_renewal_after_free_trial_with_checkout_during_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month
        .start_date(start_date)
        .on_checkout()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;

    // === Phase 1: TrialActive ===
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(true)
        .has_cycle_index(0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // === Phase 2: Complete checkout during trial (no charge, just save PM) ===
    let mut conn = env.conn().await;
    let (transaction, _) = env
        .services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0, // Free trial - no charge
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed");

    assert!(
        transaction.is_none(),
        "No payment during free trial checkout"
    );

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(false)
        .has_payment_method(true);

    // === Phase 3: Process trial end → Active with first invoice ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(true)
        .has_cycle_index(0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);

    invoices.assert().invoice_at(0).has_total(4900);

    let period_end_0 = sub
        .current_period_end
        .expect("current_period_end should be set");

    // === Phase 4: First renewal (Cycle 1) ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(1);
    assert_eq!(
        sub.current_period_start, period_end_0,
        "Period start should advance to previous period end"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices.assert().invoice_at(1).has_total(4900);

    let period_end_1 = sub.current_period_end;

    // === Phase 5: Second renewal (Cycle 2) ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(2);
    assert_eq!(
        Some(sub.current_period_start),
        period_end_1,
        "Period start should advance to previous period end"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
}

// =============================================================================
// RENEWAL AFTER PAID TRIAL
// =============================================================================

/// OnCheckout + Paid Trial: checkout → trial ends → 2 renewals
/// Total: 3 invoices at the end (1 from checkout + 2 renewals)
#[rstest]
#[tokio::test]
async fn test_renewal_after_paid_trial(#[future] test_env: TestEnv) {
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
        .is_pending_activation()
        .has_pending_checkout(true);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // === Phase 2: Complete checkout with full payment ===
    let mut conn = env.conn().await;
    let (transaction, is_pending) = env
        .services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            9900, // Full $99 - paid trial
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed");

    assert!(
        transaction.is_some(),
        "Should have payment transaction for paid trial"
    );
    assert!(!is_pending, "Payment should not be pending");

    // After checkout with paid trial, subscription goes to TrialActive
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(false)
        .has_payment_method(true)
        .has_cycle_index(0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .has_total(9900)
        .check_prorated(false);

    // === Phase 3: Process trial end → Active (no new invoice) ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(true)
        .has_trial_duration(Some(7))
        .has_cycle_index(0); // Still cycle 0 for paid trial

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1); // Still 1 invoice

    let period_end_0 = sub.current_period_end;

    // === Phase 4: First renewal (Cycle 1) ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(1);
    assert_eq!(
        Some(sub.current_period_start),
        period_end_0,
        "Period start should advance to previous period end"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices.assert().invoice_at(1).has_total(9900);

    let period_end_1 = sub.current_period_end;

    // === Phase 5: Second renewal (Cycle 2) ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(2);
    assert_eq!(
        Some(sub.current_period_start),
        period_end_1,
        "Period start should advance to previous period end"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
}

// =============================================================================
// RENEWAL AFTER TRIAL EXPIRED REACTIVATION
// =============================================================================

/// TrialExpired → checkout → Active → 2 renewals
/// Verifies renewal works correctly after reactivation from expired state.
///
/// NOTE: This test does NOT seed customer payment methods initially. If the customer
/// already has a payment method on file, the subscription will auto-charge at trial end
/// (see test_renewal_after_free_trial_with_checkout_during_trial for that flow).
/// This test specifically covers the case where a new customer signs up for a trial
/// without having a saved payment method.
/// TODO : should we allow forcing a checkout even if a card is on file (or be clearer on the OnCheckout naming)
#[rstest]
#[tokio::test]
async fn test_renewal_after_trial_expired_reactivation(#[future] test_env: TestEnv) {
    let env = test_env.await;
    // Only seed the payment provider, NOT the customer payment methods
    // This simulates a new customer without a saved card
    env.seed_mock_payment_provider(false).await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(start_date)
        .on_checkout()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;

    // === Phase 1: TrialActive ===
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(true)
        .has_payment_method(false); // No payment method - customer hasn't saved a card yet

    // === Phase 2: Let trial expire without checkout ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_expired()
        .has_pending_checkout(true)
        .has_payment_method(false);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Seed customer payment methods now (simulates customer adding card during checkout)
    env.seed_customer_payment_methods().await;

    // === Phase 3: Complete checkout after expiry → Active ===
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
        .has_cycle_index(0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);

    let period_end_0 = sub.current_period_end;

    // === Phase 4: First renewal (Cycle 1) ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(1);
    assert_eq!(
        Some(sub.current_period_start),
        period_end_0,
        "Period start should advance to previous period end"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);

    let period_end_1 = sub.current_period_end;

    // === Phase 5: Second renewal (Cycle 2) ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(2);
    assert_eq!(
        Some(sub.current_period_start),
        period_end_1,
        "Period start should advance to previous period end"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
}

// =============================================================================
// ONSTART WITH AUTO-CHARGE RENEWAL
// =============================================================================

/// OnStart + Free Trial + auto-charge: trial ends → Active → 2 renewals
#[rstest]
#[tokio::test]
async fn test_onstart_free_trial_auto_charge_renewal(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(start_date)
        .on_start()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;

    // Trial state
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(false)
        .has_cycle_index(0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Trial ends → Active
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).has_total(4900);

    let period_end_0 = sub.current_period_end;

    // First renewal: cycle 1
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(1);
    assert_eq!(
        Some(sub.current_period_start),
        period_end_0,
        "Period start should advance to previous period end"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);

    let period_end_1 = sub.current_period_end;

    // Second renewal: cycle 2
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(2);
    assert_eq!(
        Some(sub.current_period_start),
        period_end_1,
        "Period start should advance to previous period end"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
}

// =============================================================================
// ONSTART SCENARIOS (Ported from test_subscription_lifecycle.rs)
// =============================================================================

/// OnStart + No Trial + charge_automatically=false
/// Expected: Active immediately, invoice created (trust-based, unpaid)
#[rstest]
#[tokio::test]
async fn test_onstart_no_trial_no_auto_charge(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;

    // Verify subscription state
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(false)
        .has_trial_duration(None);

    // Verify invoice created
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid() // Trust-based, not auto-charged
        .has_total(3500) // $35.00
        .check_prorated(false);
}

/// OnStart + No Trial + charge_automatically=true + payment method
/// Expected: Active immediately, invoice created and paid via auto-charge
#[rstest]
#[tokio::test]
async fn test_onstart_no_trial_auto_charge_with_payment_method(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .on_start()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    // Verify subscription state - payment method is set from customer's default
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(true)
        .has_trial_duration(None);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_total(3500)
        .check_prorated(false);

    // Process outbox events to trigger payment collection. TODO improve the run_outbox_and_orchestration to run the full pipeline
    env.services()
        .complete_invoice_payment(TENANT_ID, invoices[0].id, CUST_UBER_PAYMENT_METHOD_ID)
        .await
        .unwrap();
    env.run_outbox_and_orchestration().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_paid()
        .has_total(3500)
        .check_prorated(false);
}
/// OnStart + No Trial + charge_automatically=true but NO payment method on customer
/// Expected: Active immediately, invoice created but UNPAID (no payment method to charge)
#[rstest]
#[tokio::test]
async fn test_onstart_no_trial_auto_charge_without_payment_method(#[future] test_env: TestEnv) {
    let env = test_env.await;
    // Note: NOT calling seed_payments() - customer has no payment method
    env.seed_mock_payment_provider(false).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .on_start()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    // Verify subscription state - no payment method because customer has none
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(false)
        .has_trial_duration(None);

    env.run_outbox_and_orchestration().await;

    // Invoice created but unpaid - no payment method available for auto-charge
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_total(3500)
        .check_prorated(false);
}

/// OnStart + Free Trial + charge_automatically=false
/// Expected: TrialActive → (trial ends) → Active with invoice
#[rstest]
#[tokio::test]
async fn test_onstart_free_trial_no_auto_charge(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month, 14-day free trial
        .on_start()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;

    // === Phase 1: Trial Active ===
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(false)
        .has_payment_method(false)
        .has_trial_duration(Some(14));

    // No invoices during free trial
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // === Phase 2: Process trial end ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(false)
        .has_trial_duration(Some(14));

    // Invoice created after trial ends (trust-based billing)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_total(4900) // $49.00
        .check_prorated(false);
}

/// OnStart + Paid Trial
/// Expected: TrialActive immediately + FULL invoice (NOT prorated)
/// Paid trial = feature resolution phase only, billing is normal from day 1
#[rstest]
#[tokio::test]
async fn test_onstart_paid_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID) // $99/month, 7-day PAID trial
        .on_start()
        .trial_days(7)
        .no_auto_charge()
        .create(env.services())
        .await;

    // Verify subscription state - should be TrialActive with immediate FULL invoice
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(false)
        .has_payment_method(false)
        .has_trial_duration(Some(7));

    // Paid trial should create FULL invoice immediately (NOT prorated)
    // Trial only affects feature resolution, not billing
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_total(9900) // Full $99/month - NOT prorated
        .check_prorated(false);

    // === Process trial end ===
    // Trial ends but no new invoice - already billed at creation
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(false)
        .has_trial_duration(Some(7));

    // Still only 1 invoice - paid trials don't create new invoice at trial end
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
}

// =============================================================================
// ONCHECKOUT SCENARIOS (Ported from test_subscription_lifecycle.rs)
// =============================================================================

/// OnCheckout + No Trial
/// Expected: PendingActivation → (checkout) → Active with paid invoice
#[rstest]
#[tokio::test]
async fn test_oncheckout_no_trial_full_flow(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    // === Phase 1: PendingActivation ===
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_pending_activation()
        .has_pending_checkout(true)
        .has_payment_method(true)
        .has_trial_duration(None);

    // No invoice before checkout
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Cycle processing should NOT activate the subscription
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_pending_activation();

    // === Phase 2: Complete checkout ===
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

    // === Phase 3: Verify Active state ===
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(true)
        .has_trial_duration(None);

    // Invoice should exist - payment_status remains Unpaid until webhook processes it
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid() // Updated via webhook, not sync
        .has_total(3500)
        .check_prorated(false);

    // Process outbox events to trigger payment collection
    env.run_outbox_and_orchestration().await;
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_paid()
        .has_total(3500)
        .check_prorated(false);
}
