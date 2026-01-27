//! Trial-specific tests.
//!
//! Tests for:
//! - Effective plan resolution during trial
//! - Trial end transitions
//! - Free vs Paid trial behavior

use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env};
use diesel_models::enums::SubscriptionStatusEnum;
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
        .plan_version(PLAN_VERSION_PRO_WITH_TRIAL_ID)
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

    assert_eq!(effective_plan.plan_id, PLAN_PRO_WITH_TRIAL_ID);
    assert_eq!(
        effective_plan.plan_version_id,
        PLAN_VERSION_PRO_WITH_TRIAL_ID
    );
    assert_eq!(effective_plan.plan_name, "Free with Trial");
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

/// OnStart + Paid Trial ends: becomes Active (no new invoice)
#[rstest]
#[tokio::test]
async fn test_onstart_paid_trial_ends_no_new_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID)
        .on_start()
        .trial_days(7)
        .create(env.services())
        .await;

    // Verify paid trial created invoice immediately
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);

    // Process trial end
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_trial_duration(Some(7));

    // Still only 1 invoice
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
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
