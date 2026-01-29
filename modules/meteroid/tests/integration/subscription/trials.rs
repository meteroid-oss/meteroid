//! Trial-specific tests.
//!
//! Tests for:
//! - Effective plan resolution during trial
//! - Trial end transitions
//! - Free vs Paid trial behavior

use chrono::NaiveDate;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env};
use diesel_models::enums::{CycleActionEnum, SubscriptionStatusEnum};
use meteroid_store::domain::subscription_trial::EffectivePlanSource;

// =============================================================================
// EFFECTIVE PLAN RESOLUTION TESTS
// =============================================================================

/// During trial with trialing_plan configured: effective plan = trialing_plan
#[rstest]
#[tokio::test]
async fn test_trial_uses_trialing_plan_when_configured(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // PLAN_VERSION_PRO_WITH_TRIAL_ID has trialing_plan_id pointing to Enterprise
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PRO_WITH_TRIAL_ID)
        .on_start()
        .trial_days(7)
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active();

    // Resolve effective plan - should be Enterprise (trialing plan)
    let mut conn = env.conn().await;
    let effective_plan = env
        .services()
        .get_subscription_effective_plan(&mut conn, TENANT_ID, sub_id)
        .await
        .expect("Failed to get effective plan");

    assert_eq!(effective_plan.plan_id, PLAN_ENTERPRISE_ID);
    assert_eq!(effective_plan.plan_version_id, PLAN_VERSION_ENTERPRISE_ID);
    assert_eq!(effective_plan.plan_name, "Enterprise");
    assert_eq!(effective_plan.source, EffectivePlanSource::TrialingPlan);
}

/// Trial without trialing_plan: effective plan = original plan
#[rstest]
#[tokio::test]
async fn test_trial_uses_original_plan_when_no_trialing_plan(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // LeetCode plan has no trialing_plan_id configured
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .on_start()
        .trial_days(14)
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active();

    let mut conn = env.conn().await;
    let effective_plan = env
        .services()
        .get_subscription_effective_plan(&mut conn, TENANT_ID, sub_id)
        .await
        .expect("Failed to get effective plan");

    assert_eq!(effective_plan.plan_id, PLAN_LEETCODE_ID);
    assert_eq!(effective_plan.plan_version_id, PLAN_VERSION_1_LEETCODE_ID);
    assert_eq!(effective_plan.plan_name, "LeetCode");
    assert_eq!(effective_plan.source, EffectivePlanSource::OriginalPlan);
}

/// After trial ends: effective plan = original plan
#[rstest]
#[tokio::test]
async fn test_after_trial_uses_original_plan(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .trial_days(7)
        .create(env.services())
        .await;

    // Process trial end
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let mut conn = env.conn().await;
    let effective_plan = env
        .services()
        .get_subscription_effective_plan(&mut conn, TENANT_ID, sub_id)
        .await
        .expect("Failed to get effective plan");

    assert_eq!(effective_plan.plan_id, PLAN_PAID_FREE_TRIAL_ID);
    assert_eq!(
        effective_plan.plan_version_id,
        PLAN_VERSION_PAID_FREE_TRIAL_ID
    );
    assert_eq!(effective_plan.plan_name, "Paid with Free Trial");
    assert_eq!(effective_plan.source, EffectivePlanSource::OriginalPlan);
}

/// No trial: effective plan = original plan
#[rstest]
#[tokio::test]
async fn test_no_trial_uses_original_plan(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let mut conn = env.conn().await;
    let effective_plan = env
        .services()
        .get_subscription_effective_plan(&mut conn, TENANT_ID, sub_id)
        .await
        .expect("Failed to get effective plan");

    assert_eq!(effective_plan.plan_id, PLAN_LEETCODE_ID);
    assert_eq!(effective_plan.source, EffectivePlanSource::OriginalPlan);
}

// =============================================================================
// TRIAL END TRANSITION TESTS
// =============================================================================

/// OnStart + Free Trial ends: becomes Active with invoice
#[rstest]
#[tokio::test]
async fn test_onstart_free_trial_ends_becomes_active_with_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;

    // Verify trial state
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active();

    // Process trial end
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_trial_duration(Some(14));

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_total(4900); // $49.00
}

/// OnCheckout + Free Trial ends without checkout: becomes TrialExpired
#[rstest]
#[tokio::test]
async fn test_oncheckout_free_trial_ends_without_checkout_becomes_expired(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_checkout()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;

    // Process trial end without completing checkout
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_expired()
        .has_pending_checkout(true)
        .has_payment_method(false);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

/// OnCheckout + Free Trial + auto-charge but no payment method: TrialExpired
/// This tests the important edge case where auto-charge is enabled but no payment
/// method exists. The subscription should still expire because can_auto_charge
/// requires BOTH charge_automatically=true AND a payment method on file.
#[rstest]
#[tokio::test]
async fn test_oncheckout_free_trial_auto_charge_no_payment_method_expires(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_checkout()
        .trial_days(14)
        .auto_charge() // enabled but no payment method
        .create(env.services())
        .await;

    // Verify initial state: TrialActive with pending_checkout
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(true)
        .has_payment_method(false);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Process trial end
    env.process_cycles().await;

    // Should be TrialExpired because even with auto-charge enabled,
    // there's no payment method on file
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_expired()
        .has_pending_checkout(true)
        .has_payment_method(false);

    // No invoices created (trial expired, not activated)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

/// OnStart + Paid Trial: trial end doesn't create invoice, but renewal does.
///
/// Key insight: Paid trials decouple billing from trial status.
/// - Billing: normal monthly cycle (RenewSubscription)
/// - Trial status: handled by scheduled EndTrial event
///
/// Timeline for 7-day paid trial with monthly billing starting Jan 1:
/// - Day 0 (Jan 1): Invoice 1 created, status = TrialActive
/// - Day 7 (Jan 8): Trial ends via scheduled event, status = Active, NO new invoice
/// - Day 31 (Feb 1): Billing cycle ends, Invoice 2 created via RenewSubscription
///
/// When process_cycles() runs, both events fire (since all dates are in the past).
#[rstest]
#[tokio::test]
async fn test_onstart_paid_trial_billing_cycle(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID) // $99/month, paid trial
        .start_date(start_date)
        .on_start()
        .trial_days(7)
        .no_auto_charge()
        .create(env.services())
        .await;

    // === Phase 1: Initial state - TrialActive with 1 invoice ===
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Initial state")
        .is_trial_active()
        .has_next_action(Some(CycleActionEnum::RenewSubscription)) // Paid trial uses normal billing
        .has_trial_duration(Some(7))
        .has_cycle_index(0);

    // First invoice: covers the first billing period (Jan 1 - Feb 1)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("Invoice 1: at creation")
        .is_finalized_unpaid()
        .has_total(9900)
        .has_invoice_date(start_date) // Jan 1
        .has_period(start_date, NaiveDate::from_ymd_opt(2024, 2, 1).unwrap());

    // === Phase 2: process_cycles fires both trial end AND billing renewal ===
    // Since we're running in 2026, both Jan 8 (trial end) and Feb 1 (renewal) are past
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("After process_cycles")
        .is_active() // Trial ended via scheduled event
        .has_trial_duration(Some(7))
        .has_cycle_index(1); // Cycle advanced due to renewal

    // Two invoices: creation + renewal at month-end
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);

    // Invoice 1: still the original at Jan 1
    invoices
        .assert()
        .invoice_at(0)
        .with_context("Invoice 1: at creation")
        .has_invoice_date(start_date);

    // Invoice 2: at Feb 1 (billing cycle end), NOT at Jan 8 (trial end)
    // This proves trial end doesn't create an invoice - the renewal does
    let expected_renewal_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    invoices
        .assert()
        .invoice_at(1)
        .with_context("Invoice 2: at billing cycle end, not trial end")
        .is_finalized_unpaid()
        .has_total(9900)
        .has_invoice_date(expected_renewal_date)
        .has_period(
            expected_renewal_date,
            NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        );

    // Verify: no invoice on Jan 8 (trial end date)
    let trial_end_date = NaiveDate::from_ymd_opt(2024, 1, 8).unwrap();
    for invoice in invoices.iter() {
        assert_ne!(
            invoice.invoice_date, trial_end_date,
            "No invoice should be dated at trial end ({})",
            trial_end_date
        );
    }
}

// =============================================================================
// FREE PLAN TESTS
// =============================================================================

/// Free plan with trial: never bills, goes to Active (not TrialExpired)
#[rstest]
#[tokio::test]
async fn test_free_plan_with_trial_never_bills(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // PLAN_VERSION_PRO_WITH_TRIAL_ID is a Free plan type
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PRO_WITH_TRIAL_ID)
        .on_start()
        .trial_days(7)
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active();

    // No invoices during trial
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Process trial end
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active() // NOT TrialExpired!
        .has_trial_duration(Some(7));

    // Still no invoices (free plan)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

// =============================================================================
// ONSTART VS ONCHECKOUT COMPARISON
// =============================================================================

/// Compare OnStart vs OnCheckout behavior after trial ends
#[rstest]
#[tokio::test]
async fn test_onstart_vs_oncheckout_trial_end_comparison(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;

    // OnStart subscription
    let onstart_sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;

    // OnCheckout subscription (different customer)
    let oncheckout_sub_id = subscription()
        .customer(CUST_SPOTIFY_ID)
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_checkout()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;

    // Both start in TrialActive
    let onstart_sub = env.get_subscription(onstart_sub_id).await;
    let oncheckout_sub = env.get_subscription(oncheckout_sub_id).await;
    onstart_sub
        .assert()
        .has_status(SubscriptionStatusEnum::TrialActive);
    oncheckout_sub
        .assert()
        .has_status(SubscriptionStatusEnum::TrialActive);

    // Process trial end
    env.process_cycles().await;

    // OnStart → Active (trust-based)
    let onstart_sub = env.get_subscription(onstart_sub_id).await;
    onstart_sub
        .assert()
        .with_context("OnStart after trial")
        .is_active();

    let onstart_invoices = env.get_invoices(onstart_sub_id).await;
    assert!(!onstart_invoices.is_empty(), "OnStart should have invoice");

    // OnCheckout → TrialExpired (checkout required)
    let oncheckout_sub = env.get_subscription(oncheckout_sub_id).await;
    oncheckout_sub
        .assert()
        .with_context("OnCheckout after trial")
        .is_trial_expired()
        .has_pending_checkout(true);

    let oncheckout_invoices = env.get_invoices(oncheckout_sub_id).await;
    oncheckout_invoices.assert().assert_empty();
}

// =============================================================================
// ONSTART FREE TRIAL WITH AUTO-CHARGE
// =============================================================================

/// Test OnStart with free trial and auto-charge enabled.
/// Trial should start immediately, and when it ends, subscription becomes Active (trust-based).
#[rstest]
#[tokio::test]
async fn test_onstart_free_trial_with_auto_charge_enabled(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .trial_days(14)
        .auto_charge() // Auto-charge enabled (but no payment method)
        .create(env.services())
        .await;

    // Subscription should be TrialActive
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("OnStart with free trial should start in TrialActive")
        .is_trial_active()
        .has_pending_checkout(false); // OnStart is trust-based

    // No invoice yet (free trial)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Process cycle to end trial
    env.process_cycles().await;

    // OnStart with auto-charge but no payment method should still become Active (trust-based)
    // because OnStart is fundamentally trust-based - auto_charge affects invoices, not subscription status
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context(
            "OnStart should become Active after trial (trust-based, regardless of auto-charge)",
        )
        .is_active();

    // Should have an invoice after trial ends
    let invoices = env.get_invoices(sub_id).await;
    assert!(!invoices.is_empty(), "Should have invoice after trial ends");
}

// =============================================================================
// PORTED TESTS FROM test_trials.rs
// =============================================================================

/// Paid plan with free trial on OnStart becomes Active when trial ends
/// even without a payment method. Invoice is created, subscription continues.
/// This is the expected behavior: OnStart = subscription already activated, just bill.
#[rstest]
#[tokio::test]
async fn test_paid_plan_free_trial_onstart_no_payment_method_becomes_active(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;

    // Create a subscription on a paid plan with free trial, no payment method
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;

    // Verify initial state
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active().has_payment_method(false);

    // Process cycle transitions to end the trial
    env.process_cycles().await;

    // Get updated subscription
    let sub = env.get_subscription(sub_id).await;

    // Should be Active because:
    // - OnStart means subscription was already activated
    // - Trial ending just triggers billing, not a "needs checkout" state
    // - Invoice is created, subscription continues regardless of payment
    sub.assert()
        .with_context("OnStart subscription should be Active after trial ends (invoice sent, service continues)")
        .is_active();

    // Verify an invoice was created
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).has_total(4900); // $49.00
}

/// OnCheckout + auto-charge + no payment method → TrialExpired
/// When a paid trial ends with OnCheckout activation, auto-charge enabled,
/// but no payment method on file, the subscription should transition to TrialExpired
/// because can_auto_charge requires both charge_automatically AND payment_method.
#[rstest]
#[tokio::test]
async fn test_oncheckout_trial_with_payment_method_becomes_active(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Seed the mock payment provider (payments will succeed)
    env.seed_mock_payment_provider(false).await;

    // Create a subscription with OnCheckout activation and auto-charge enabled
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_checkout()
        .trial_days(14)
        .auto_charge() // enabled but no payment method
        .create(env.services())
        .await;

    // Verify initial state
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active();

    // Note: At this point the customer doesn't have a payment method yet.
    // In a real scenario, they would complete checkout before trial ends.
    // For this test, without a payment method, it should still go to TrialExpired
    // because can_auto_charge requires both charge_automatically AND payment_method.

    // Process cycle transitions to end the trial
    env.process_cycles().await;

    // Get updated subscription
    let sub = env.get_subscription(sub_id).await;

    // Should be TrialExpired because even with auto-charge enabled,
    // there's no payment method on file yet (customer hasn't completed checkout)
    sub.assert()
        .with_context("OnCheckout subscription without payment method should be TrialExpired even with auto-charge")
        .is_trial_expired()
        .has_pending_checkout(true);
}

// =============================================================================
// PAID TRIAL INVOICE GENERATION BUG REPRODUCTION
// =============================================================================

/// BUG REPRODUCTION: Paid plan with monthly rate and 90-day paid trial.
///
/// This test reproduces a bug where one invoice is missing from the first 4 generated invoices
/// when you have:
/// - A paid plan with a monthly rate
/// - A paid trial giving another plan's features for 90 days
///
/// Expected behavior:
/// - Cycle 0 (creation): 1 invoice created immediately (paid trial bills from day 1)
/// - Process cycle 1 (month 1 renewal): 2nd invoice
/// - Process cycle 2 (month 2 renewal): 3rd invoice
/// - Process cycle 3 (trial ends ~90 days, month 3 renewal): 4th invoice
///
/// The trial spans approximately 3 billing periods. After 4 process_cycles calls,
/// we should have 4 invoices total.
///
/// BUG: Currently one invoice is missing because EndTrial incorrectly sets
/// should_bill=false for paid trials, skipping the invoice at trial end.
#[rstest]
#[tokio::test]
async fn test_paid_trial_90_days_generates_all_invoices(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Create a subscription with:
    // - Paid plan with monthly rate ($99/month)
    // - 90-day paid trial giving Enterprise features
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID) // $99/month, paid trial with trialing_plan
        .start_date(start_date)
        .on_start()
        .trial_days(90) // 90-day paid trial (overrides default 7-day)
        .no_auto_charge()
        .create(env.services())
        .await;

    // === Initial state: TrialActive with 1 invoice ===
    // Paid trial bills immediately at creation
    // Note: Paid trials use RenewSubscription (not EndTrial) for billing cycles
    // Trial end is handled via scheduled event, not cycle action
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Initial state")
        .has_status(SubscriptionStatusEnum::TrialActive)
        .has_next_action(Some(CycleActionEnum::RenewSubscription)) // Paid trial uses normal renewal
        .has_trial_duration(Some(90))
        .has_cycle_index(0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("Invoice at creation")
        .is_finalized_unpaid()
        .has_total(9900); // $99.00

    // === Process cycle 1: First renewal during trial ===
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 2, "After cycle 1: should have 2 invoices");

    // === Process cycle 2: Second renewal ===
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 3, "After cycle 2: should have 3 invoices");

    // === Process cycle 3: Third renewal (trial should end around here) ===
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 4, "After cycle 3: should have 4 invoices");

    // === Process cycle 4: Fourth renewal ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().with_context("After 4 cycles").is_active(); // Should be active after 90-day trial ends

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 5, "After cycle 4: should have 5 invoices");

    // Debug: Print invoice dates to see the gap
    println!("Invoice dates (should be consecutive months starting Jan 1, 2024):");
    for (i, invoice) in invoices.iter().enumerate() {
        println!("  Invoice {}: {}", i, invoice.invoice_date);
    }

    // Expected invoice dates for monthly billing starting Jan 1, 2024:
    // Invoice 0: 2024-01-01 (creation)
    // Invoice 1: 2024-02-01 (1st renewal)
    // Invoice 2: 2024-03-01 (2nd renewal)
    // Invoice 3: 2024-04-01 (3rd renewal, trial ends around here)
    // Invoice 4: 2024-05-01 (4th renewal)
    //
    // BUG: One invoice is missing - check the dates to see the gap
    let expected_dates = vec![
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 4, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 5, 1).unwrap(),
    ];

    for (i, expected_date) in expected_dates.iter().enumerate() {
        assert!(
            invoices.get(i).is_some(),
            "Missing invoice {}: expected date {}",
            i,
            expected_date
        );
        assert_eq!(
            invoices[i].invoice_date, *expected_date,
            "Invoice {} has wrong date: expected {}, got {}",
            i, expected_date, invoices[i].invoice_date
        );
    }
}

/// Paid plan with FREE trial (90 days).
/// - No invoices during trial
/// - After trial ends (90 days), billing starts
/// - Each cycle after trial generates an invoice
#[rstest]
#[tokio::test]
async fn test_paid_free_trial_90_days_generates_all_invoices(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month, FREE trial
        .start_date(start_date)
        .on_start()
        .trial_days(90)
        .no_auto_charge()
        .create(env.services())
        .await;

    // === Initial state: TrialActive with NO invoice ===
    // Free trial = no billing until trial ends
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Initial state")
        .is_trial_active() // Free trial uses EndTrial action
        .has_trial_duration(Some(90))
        .has_cycle_index(0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(0); // No invoice during free trial

    // === Process cycle 1: Trial ends (90 days), first invoice created ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("After trial ends")
        .is_active()
        .has_cycle_index(0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    // First invoice is for the period starting at trial end (90 days from Jan 1 = Mar 31)
    assert_eq!(
        invoices[0].invoice_date,
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
        "First invoice should be at trial end date"
    );

    // === Process cycle 2: First renewal ===
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);

    // === Process cycle 3: Second renewal ===
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);

    // === Process cycle 4: Third renewal ===
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(4);

    // Verify invoice dates are consecutive months starting from trial end
    println!("Invoice dates for free trial (should start at trial end Apr 1, 2024):");
    for (i, invoice) in invoices.iter().enumerate() {
        println!("  Invoice {}: {}", i, invoice.invoice_date);
    }

    // 90 days from Jan 1 = Mar 31, then monthly billing anchored to that day
    let expected_dates = vec![
        NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(), // Trial end (Jan 1 + 90 days)
        NaiveDate::from_ymd_opt(2024, 4, 30).unwrap(), // +1 month (Apr 30 because Apr has 30 days)
        NaiveDate::from_ymd_opt(2024, 5, 31).unwrap(), // +1 month
        NaiveDate::from_ymd_opt(2024, 6, 30).unwrap(), // +1 month (Jun has 30 days)
    ];

    for (i, expected_date) in expected_dates.iter().enumerate() {
        assert_eq!(
            invoices[i].invoice_date, *expected_date,
            "Invoice {} has wrong date: expected {}, got {}",
            i, expected_date, invoices[i].invoice_date
        );
    }
}

/// Free plan with trial (edge case).
/// - Free plan = no billing ever
/// - Trial on free plan just delays activation but no invoices regardless
#[rstest]
#[tokio::test]
async fn test_free_plan_with_trial_generates_no_invoices(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_FREE_ID) // Free plan
        .start_date(start_date)
        .on_start()
        .trial_days(90)
        .no_auto_charge()
        .create(env.services())
        .await;

    // === Initial state ===
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Initial state")
        .has_trial_duration(Some(90));

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(0); // No invoice - free plan

    // === Process cycles ===
    env.process_cycles().await;
    env.process_cycles().await;
    env.process_cycles().await;
    env.process_cycles().await;

    // After 4 cycles, should still have no invoices (free plan)
    let invoices = env.get_invoices(sub_id).await;
    invoices
        .assert()
        .with_context("Free plan should never generate invoices")
        .has_count(0);

    // Subscription should be active
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();
}
