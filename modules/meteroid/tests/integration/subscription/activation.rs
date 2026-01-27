//! Subscription activation tests using parameterized test cases.
//!
//! Tests the initial state of subscriptions based on:
//! - Activation condition (OnStart, OnCheckout, Manual)
//! - Trial configuration (None, Free, Paid)
//! - Auto-charge setting

use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env};
use diesel_models::enums::{CycleActionEnum, SubscriptionStatusEnum};
use meteroid_store::domain::SubscriptionActivationCondition;

// =============================================================================
// ONSTART ACTIVATION TESTS
// =============================================================================

/// OnStart + No Trial: Should be Active immediately with invoice
#[rstest]
#[tokio::test]
async fn test_onstart_no_trial_is_active_immediately(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(false);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_total(3500);
}

/// OnStart + Free Trial: Should be TrialActive, no invoice yet
#[rstest]
#[tokio::test]
async fn test_onstart_free_trial_starts_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month with free trial
        .on_start()
        .trial_days(14)
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(false)
        .has_payment_method(false)
        .has_trial_duration(Some(14));

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

/// OnStart + Paid Trial: Should be TrialActive with immediate FULL invoice
#[rstest]
#[tokio::test]
async fn test_onstart_paid_trial_bills_immediately(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID) // $99/month with paid trial
        .on_start()
        .trial_days(7)
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(false)
        .has_trial_duration(Some(7));

    // Paid trial creates FULL invoice immediately (NOT prorated)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_total(9900)
        .check_prorated(false);
}

// =============================================================================
// ONCHECKOUT ACTIVATION TESTS
// =============================================================================

/// OnCheckout + No Trial: Should be PendingActivation
#[rstest]
#[tokio::test]
async fn test_oncheckout_no_trial_starts_pending(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_pending_activation()
        .has_pending_checkout(true)
        .has_payment_method(false);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

/// OnCheckout + Free Trial: Should be TrialActive with pending_checkout=true
#[rstest]
#[tokio::test]
async fn test_oncheckout_free_trial_starts_trial_with_pending_checkout(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_checkout()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(true) // OnCheckout sets pending_checkout even during trial
        .has_payment_method(false)
        .has_trial_duration(Some(14))
        .has_next_action(Some(CycleActionEnum::EndTrial)); // Trial will end at next cycle

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

/// OnCheckout + Paid Trial: Should be PendingActivation (payment required to start)
#[rstest]
#[tokio::test]
async fn test_oncheckout_paid_trial_starts_pending(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID)
        .on_checkout()
        .trial_days(7)
        .auto_charge()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_pending_activation()
        .has_pending_checkout(true)
        .has_payment_method(false)
        .has_trial_duration(Some(7));

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

// =============================================================================
// MANUAL ACTIVATION TESTS
// =============================================================================

/// Manual + No Trial: Should be PendingActivation without pending_checkout
#[rstest]
#[tokio::test]
async fn test_manual_no_trial_starts_pending_without_checkout_flag(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .manual()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_pending_activation()
        .has_pending_checkout(false) // Manual doesn't use pending_checkout
        .has_payment_method(false);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

// =============================================================================
// CYCLE PROCESSING BEHAVIOR TESTS
// =============================================================================

/// OnCheckout without trial stays pending after cycle processing
#[rstest]
#[tokio::test]
async fn test_oncheckout_stays_pending_after_cycle_processing(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    // Process cycles - should NOT activate
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_pending_activation()
        .has_pending_checkout(true);
}

/// Manual activation stays pending after cycle processing
#[rstest]
#[tokio::test]
async fn test_manual_stays_pending_after_cycle_processing(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .manual()
        .no_trial()
        .create(env.services())
        .await;

    // Process cycles - should NOT activate
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_pending_activation();
}

// =============================================================================
// PARAMETERIZED ACTIVATION MATRIX TEST
// =============================================================================

/// Parameterized test for initial subscription state based on activation + trial config.
#[rstest]
#[case::onstart_no_trial(
    SubscriptionActivationCondition::OnStart,
    PLAN_VERSION_1_LEETCODE_ID, // No trial plan
    None,
    false,
    SubscriptionStatusEnum::Active,
    Some(CycleActionEnum::RenewSubscription),
    false,
    true // has invoice
)]
#[case::onstart_free_trial(
    SubscriptionActivationCondition::OnStart,
    PLAN_VERSION_PAID_FREE_TRIAL_ID, // Free trial plan
    Some(14),
    false,
    SubscriptionStatusEnum::TrialActive,
    Some(CycleActionEnum::EndTrial),
    false,
    false // no invoice during free trial
)]
#[case::oncheckout_no_trial(
    SubscriptionActivationCondition::OnCheckout,
    PLAN_VERSION_1_LEETCODE_ID, // No trial plan
    None,
    true,
    SubscriptionStatusEnum::PendingActivation,
    None,
    true, // pending_checkout
    false // no invoice
)]
#[case::oncheckout_free_trial(
    SubscriptionActivationCondition::OnCheckout,
    PLAN_VERSION_PAID_FREE_TRIAL_ID, // Free trial plan
    Some(14),
    true,
    SubscriptionStatusEnum::TrialActive,
    Some(CycleActionEnum::EndTrial),
    true, // pending_checkout even during trial
    false // no invoice
)]
#[case::manual_no_trial(
    SubscriptionActivationCondition::Manual,
    PLAN_VERSION_1_LEETCODE_ID, // No trial plan
    None,
    true,
    SubscriptionStatusEnum::PendingActivation,
    None,
    false, // manual doesn't use pending_checkout
    false // no invoice
)]
#[tokio::test]
async fn test_activation_matrix(
    #[future] test_env: TestEnv,
    #[case] activation: SubscriptionActivationCondition,
    #[case] plan_version_id: common_domain::ids::PlanVersionId,
    #[case] trial_days: Option<u32>,
    #[case] needs_payment_provider: bool,
    #[case] expected_status: SubscriptionStatusEnum,
    #[case] expected_action: Option<CycleActionEnum>,
    #[case] expected_pending_checkout: bool,
    #[case] expected_has_invoice: bool,
) {
    let env = test_env.await;

    if needs_payment_provider {
        env.seed_mock_payment_provider(false).await;
    }

    let mut builder = subscription()
        .plan_version(plan_version_id)
        .activation(activation);

    if let Some(days) = trial_days {
        builder = builder.trial_days(days);
    }

    let sub_id = builder.create(env.services()).await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .has_status(expected_status)
        .has_next_action(expected_action)
        .has_pending_checkout(expected_pending_checkout);

    let invoices = env.get_invoices(sub_id).await;
    if expected_has_invoice {
        assert!(!invoices.is_empty(), "Expected invoice to be created");
    } else {
        invoices.assert().assert_empty();
    }
}

// =============================================================================
// ONSTART VS ONCHECKOUT COMPARISON (NO TRIAL)
// =============================================================================

/// Compare OnStart vs OnCheckout behavior without trial.
/// OnStart without trial: Active immediately with invoice.
/// OnCheckout without trial: PendingActivation without invoice.
#[rstest]
#[tokio::test]
async fn test_onstart_vs_oncheckout_no_trial_comparison(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;

    // Create OnStart subscription (no trial)
    let onstart_sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial() // Explicitly disable trial at the subscription level
        .no_auto_charge()
        .create(env.services())
        .await;

    // Create OnCheckout subscription (no trial)
    let oncheckout_sub_id = subscription()
        .customer(CUST_SPOTIFY_ID)
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_checkout()
        .no_trial() // Explicitly disable trial
        .auto_charge()
        .create(env.services())
        .await;

    // OnStart without trial: Active immediately
    let onstart_sub = env.get_subscription(onstart_sub_id).await;
    onstart_sub
        .assert()
        .with_context("OnStart without trial should be Active immediately")
        .is_active();

    // OnCheckout without trial: PendingActivation
    let oncheckout_sub = env.get_subscription(oncheckout_sub_id).await;
    oncheckout_sub
        .assert()
        .with_context("OnCheckout without trial should be PendingActivation")
        .is_pending_activation();

    // OnStart should have an invoice
    let onstart_invoices = env.get_invoices(onstart_sub_id).await;
    assert!(
        !onstart_invoices.is_empty(),
        "OnStart should have an invoice immediately"
    );

    // OnCheckout should NOT have an invoice
    let oncheckout_invoices = env.get_invoices(oncheckout_sub_id).await;
    oncheckout_invoices.assert().assert_empty();
}

// =============================================================================
// ONSTART WITH AUTO-CHARGE TESTS
// =============================================================================

/// Test OnStart with auto-charge enabled.
/// Subscription should be Active immediately with invoice (trust-based).
#[rstest]
#[tokio::test]
async fn test_onstart_with_auto_charge_and_payment_method(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial() // Explicitly disable trial
        .auto_charge() // Auto-charge enabled
        .create(env.services())
        .await;

    // Subscription should be Active immediately
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("OnStart subscription should be Active immediately")
        .is_active()
        .has_pending_checkout(false); // OnStart is trust-based

    // Should have an invoice
    let invoices = env.get_invoices(sub_id).await;
    assert!(
        !invoices.is_empty(),
        "OnStart should create an invoice immediately"
    );
}

/// Test OnStart without auto-charge (trust-based billing).
/// Invoice is created but no automatic payment attempt.
#[rstest]
#[tokio::test]
async fn test_onstart_without_auto_charge_trust_based(#[future] test_env: TestEnv) {
    let env = test_env.await;
    // Note: OnStart subscriptions don't require mock payment provider

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial() // Explicitly disable trial
        .no_auto_charge() // No auto-charge (trust-based)
        .create(env.services())
        .await;

    // Subscription should be Active immediately
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("OnStart subscription should be Active immediately")
        .is_active()
        .has_pending_checkout(false); // OnStart is trust-based

    // Should have an invoice (trust-based, payment will be collected manually or later)
    let invoices = env.get_invoices(sub_id).await;
    assert!(!invoices.is_empty(), "OnStart should create an invoice");
}

// =============================================================================
// MANUAL ACTIVATION DETAILED TESTS
// =============================================================================

/// Test Manual activation without auto-charge.
/// Subscription starts as PendingActivation, waiting for manual activation.
#[rstest]
#[tokio::test]
async fn test_manual_activation_starts_pending(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .manual()
        .no_auto_charge()
        .create(env.services())
        .await;

    // Subscription should be PendingActivation
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Manual activation subscription should start in PendingActivation")
        .is_pending_activation()
        .has_pending_checkout(false); // Manual is different from OnCheckout

    // No invoices should be created until activation
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

// =============================================================================
// MANUAL ACTIVATION CYCLE PROCESSING
// =============================================================================

/// Manual + No Trial: stays PendingActivation even after cycle processing
/// Verifies that Manual activation requires explicit admin action, not time-based triggers
#[rstest]
#[tokio::test]
async fn test_manual_no_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .manual()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;

    // Phase 1: Verify initial PendingActivation state
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_pending_activation()
        .has_pending_checkout(false) // Manual doesn't use pending_checkout
        .has_payment_method(false)
        .has_trial_duration(None);

    // No invoices before manual activation
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Cycle processing should NOT change state for Manual activation
    env.process_cycles().await;

    // Should STILL be PendingActivation - awaiting admin activation
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_pending_activation()
        .has_pending_checkout(false);

    // Still no invoices
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}
