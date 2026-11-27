//! Subscription lifecycle and renewal tests.
//!
//! Tests for:
//! - Renewal cycles
//! - Period transitions
//! - Multi-cycle scenarios
//!
//! Ported from test_subscription_lifecycle.rs

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{NaiveDate, NaiveTime};
use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId};
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageClient, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::{BillingType, Invoice, UsagePeriod};
use meteroid_store::services::CheckoutPaymentOutcome;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{
    InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env,
    test_env_with_seed_and_usage,
};
use crate::meteroid_it::container::SeedLevel;
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use diesel_models::enums::SubscriptionStatusEnum;

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
    sub.assert().is_trial_active().has_pending_checkout(true);
    assert_eq!(env.billed_cycle(sub_id).await, 0);

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
        .has_resolved_payment_method(&env, true)
        .await;

    // === Phase 3: Process trial end → Active: its checkout settled the start during the trial ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_pending_checkout(false);

    // Invoice created but payment not yet processed
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);

    // === Phase 3b: Payment settles → Active ===
    env.run_outbox_and_orchestration().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_resolved_payment_method(&env, true)
        .await;
    assert_eq!(env.billed_cycle(sub_id).await, 0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);

    invoices.assert().invoice_at(0).has_total(4900);

    let period_end_0 = sub
        .current_period_end
        .expect("current_period_end should be set");

    // === Phase 4: First renewal (Cycle 1) ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();
    assert_eq!(env.billed_cycle(sub_id).await, 1);
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
    sub.assert().is_active();
    assert_eq!(env.billed_cycle(sub_id).await, 2);
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

/// OnCheckout + Paid Trial: checkout → renewals
///
/// With paid trials, billing is decoupled from trial status:
/// - Checkout creates invoice 1 (trial active, bills immediately)
/// - First process_cycles fires both trial end AND month-1 renewal → invoice 2
/// - Each subsequent process_cycles creates another renewal invoice
///
/// Total: 4 invoices at the end (1 from checkout + 3 renewals)
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
        .with_context("Phase 1: PendingActivation")
        .is_pending_activation()
        .has_pending_checkout(true);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // === Phase 2: Complete checkout with full payment ===
    let mut conn = env.conn().await;
    let (transaction, payment_outcome) = env
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
    assert_eq!(
        payment_outcome,
        CheckoutPaymentOutcome::Settled,
        "Card payment should settle inline"
    );

    // After checkout: TrialActive with 1 invoice
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Phase 2: After checkout")
        .is_trial_active()
        .has_pending_checkout(false)
        .has_cycle_index(0)
        .has_resolved_payment_method(&env, true)
        .await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("Invoice 1: at checkout")
        .has_total(9900)
        .has_billing_period_start(start_date)
        .check_prorated(false);

    // === Phase 3: First process_cycles - trial ends AND renewal fires ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Phase 3: After first process_cycles")
        .is_active()
        .has_pending_checkout(false)
        .has_cycle_index(1) // Cycle advanced due to renewal
        .has_resolved_payment_method(&env, true)
        .await;
    assert_eq!(env.trial_days(sub_id).await, Some(7));

    // 2 invoices: checkout + first renewal
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("Invoice 2: first renewal")
        .has_total(9900)
        .has_billing_period_start(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap());

    let period_end_1 = sub.current_period_end;

    // === Phase 4: Second renewal (Cycle 2) ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Phase 4: Second renewal")
        .is_active()
        .has_cycle_index(2);
    assert_eq!(
        Some(sub.current_period_start),
        period_end_1,
        "Period start should advance to previous period end"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    invoices
        .assert()
        .invoice_at(2)
        .has_total(9900)
        .has_billing_period_start(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap());

    let period_end_2 = sub.current_period_end;

    // === Phase 5: Third renewal (Cycle 3) ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Phase 5: Third renewal")
        .is_active()
        .has_cycle_index(3);
    assert_eq!(
        Some(sub.current_period_start),
        period_end_2,
        "Period start should advance to previous period end"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(4);
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
    sub.assert().is_trial_active().has_pending_checkout(true);

    // === Phase 2: Let trial expire without checkout ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_expired().has_pending_checkout(true);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Seed customer payment methods now (simulates customer adding card during checkout)
    env.seed_customer_payment_methods().await;

    // === Phase 3: Complete checkout after expiry → Active ===
    let mut conn = env.conn().await;
    let (transaction, payment_outcome) = env
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
    assert_eq!(
        payment_outcome,
        CheckoutPaymentOutcome::Settled,
        "Card payment should settle inline"
    );

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_resolved_payment_method(&env, true)
        .await;
    assert_eq!(env.billed_cycle(sub_id).await, 0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);

    let period_end_0 = sub.current_period_end;

    // === Phase 4: First renewal (Cycle 1) ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();
    assert_eq!(env.billed_cycle(sub_id).await, 1);
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
    sub.assert().is_active();
    assert_eq!(env.billed_cycle(sub_id).await, 2);
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
    sub.assert().is_trial_active().has_pending_checkout(false);
    assert_eq!(env.billed_cycle(sub_id).await, 0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Trial ends → Active
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();
    assert_eq!(env.billed_cycle(sub_id).await, 0);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).has_total(4900);

    let period_end_0 = sub.current_period_end;

    // First renewal: cycle 1
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();
    assert_eq!(env.billed_cycle(sub_id).await, 1);
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
    sub.assert().is_active();
    assert_eq!(env.billed_cycle(sub_id).await, 2);
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
    sub.assert().is_active().has_pending_checkout(false);
    assert_eq!(env.trial_days(sub_id).await, None);

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
        .has_resolved_payment_method(&env, true)
        .await;
    assert_eq!(env.trial_days(sub_id).await, None);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_total(3500)
        .check_prorated(false);

    // Process outbox events to trigger payment collection.
    // TODO improve the run_outbox_and_orchestration to run the full pipeline
    env.services()
        .complete_invoice_payment(
            TENANT_ID,
            invoices[0].id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
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

    // Verify subscription state
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_pending_checkout(false);
    assert_eq!(env.trial_days(sub_id).await, None);

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
    sub.assert().is_trial_active().has_pending_checkout(false);
    assert_eq!(env.trial_days(sub_id).await, Some(14));

    // No invoices during free trial
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // === Phase 2: Process trial end ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_pending_checkout(false);
    assert_eq!(env.trial_days(sub_id).await, Some(14));

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

/// OnStart + Paid Trial: billing is decoupled from trial status.
///
/// Key behavior:
/// - Paid trial bills immediately using normal billing cycles (RenewSubscription)
/// - Trial status change (TrialActive → Active) is derived from the trial fact at its end
/// - Trial end does NOT create an invoice - only billing cycle renewal does
///
/// Expected timeline for 7-day trial with monthly billing:
/// - Day 0: Invoice 1 (creation), status = TrialActive
/// - Day 7: Status → Active (no invoice)
/// - Day 31: Invoice 2 (renewal)
#[rstest]
#[tokio::test]
async fn test_onstart_paid_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID) // $99/month, 7-day PAID trial
        .start_date(start_date)
        .on_start()
        .trial_days(7)
        .no_auto_charge()
        .create(env.services())
        .await;

    // === Phase 1: Initial state ===
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Initial state")
        .is_trial_active()
        .has_pending_checkout(false)
        .has_cycle_index(0);
    assert_eq!(env.trial_days(sub_id).await, Some(7));

    // Paid trial creates FULL invoice immediately (NOT prorated)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("Invoice 1: at creation")
        .is_finalized_unpaid()
        .has_total(9900) // Full $99/month
        .has_billing_period_start(start_date)
        .check_prorated(false);

    // === Phase 2: process_cycles - trial ends AND renewal fires ===
    // Both happen because dates are in the past when test runs
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("After process_cycles")
        .is_active()
        .has_pending_checkout(false)
        .has_cycle_index(1); // Cycle advanced
    assert_eq!(env.trial_days(sub_id).await, Some(7));

    // Two invoices: creation + renewal at month end
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);

    // Verify invoice 2 is at month-end, not trial-end
    let expected_renewal_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    invoices
        .assert()
        .invoice_at(1)
        .with_context("Invoice 2: at renewal")
        .is_finalized_unpaid()
        .has_total(9900)
        .has_billing_period_start(expected_renewal_date);
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
        .has_resolved_payment_method(&env, true)
        .await;
    assert_eq!(env.trial_days(sub_id).await, None);

    // No invoice before checkout
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Cycle processing should NOT activate the subscription
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_pending_activation();

    // === Phase 2: Complete checkout ===
    let mut conn = env.conn().await;
    let (transaction, payment_outcome) = env
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
    assert_eq!(
        payment_outcome,
        CheckoutPaymentOutcome::Settled,
        "Card payment should settle inline"
    );

    // === Phase 3: Verify Active state ===
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_resolved_payment_method(&env, true)
        .await;
    assert_eq!(env.trial_days(sub_id).await, None);

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

// =============================================================================
// END DATE LIFECYCLE TESTS
// =============================================================================

/// Subscription with end_date in the past (skip_past_invoices=false):
/// billing terminates at end_date via the cycle worker.
///
/// Timeline (monthly billing, billing_day=1):
/// - Creation (2024-01-01): Active, cycle_index=0, period=[Jan1, Feb1], Invoice 1 ($35)
/// - process_cycles #1: RenewSubscription fires; end_date(Feb15) truncates [Feb1, Mar1] → [Feb1, Feb15];
///   EndSubscription scheduled; Invoice 2 bills [Feb1, Feb15), its share of February
/// - process_cycles #2: EndSubscription fires → Completed;
///   No Invoice 3 (advance_period=None for completed subscriptions)
///
/// Rules: R-079, R-084
#[rstest]
#[tokio::test]
async fn test_lifecycle_end_date_billing_terminates(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    // end_date falls within the second billing period [Feb1, Mar1]
    let end_date = NaiveDate::from_ymd_opt(2024, 2, 15).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .start_date(start_date)
        .end_date(end_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // === Phase 1: Creation ===
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Phase 1: After creation")
        .is_active()
        .has_cycle_index(0)
        .has_period_start(start_date)
        .has_period_end(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap());

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("Invoice 1: first full period")
        .is_finalized_unpaid()
        .has_billing_period_start(start_date)
        .has_total(3500)
        .check_prorated(false);

    // === Phase 2: First renewal — period is truncated at end_date ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Phase 2: After first process_cycles")
        // Active + EndSubscription (not RenewSubscription) because end_date is within this period
        .has_status(SubscriptionStatusEnum::Active)
        .has_cycle_index(1)
        .has_period_start(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
        .has_period_end(end_date);

    // February's window is [Feb1, Feb15): 3500 × 14/29 = 1689.66 → 1690.
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("Invoice 2: the term's last period, cut at its end")
        .is_finalized_unpaid()
        .has_billing_period_start(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
        .has_total(1690);
    assert_eq!(
        lines_of(&invoices[0]),
        vec![line("Subscription Rate", (2024, 1, 1), (2024, 2, 1), 3500)]
    );
    assert_eq!(
        lines_of(&invoices[1]),
        vec![line("Subscription Rate", (2024, 2, 1), (2024, 2, 15), 1690)]
    );

    // === Phase 3: EndSubscription fires — subscription completes with no new invoice ===
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Phase 3: After second process_cycles")
        .has_status(SubscriptionStatusEnum::Completed);

    // No Invoice 3: when EndSubscription fires, current_period_end=None in the DB,
    // so the billing layer treats the subscription as completed and skips advance billing.
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
}

type Line = (String, NaiveDate, NaiveDate, i64);

fn line(name: &str, from: (i32, u32, u32), to: (i32, u32, u32), amount: i64) -> Line {
    let day = |(y, m, d): (i32, u32, u32)| NaiveDate::from_ymd_opt(y, m, d).unwrap();
    (name.to_string(), day(from), day(to), amount)
}

/// Each line of `invoice` as (name, start, end, subtotal), sorted.
fn lines_of(invoice: &Invoice) -> Vec<Line> {
    let mut lines: Vec<Line> = invoice
        .line_items
        .iter()
        .map(|l| (l.name.clone(), l.start_date, l.end_date, l.amount_subtotal))
        .collect();
    lines.sort();
    lines
}

fn bandwidth(usage: &MutableMockUsageClient, from: NaiveDate, to: NaiveDate, units: i64) {
    let (start, end) = (from.and_time(NaiveTime::MIN), to.and_time(NaiveTime::MIN));
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start,
            period_end: end,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(units),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod { start, end },
        },
    );
}

/// A term ending on a period boundary bills its periods whole.
/// Rules: R-084
#[rstest]
#[tokio::test]
async fn test_fixed_term_ending_on_a_boundary_bills_whole_periods(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let day = |m, d| NaiveDate::from_ymd_opt(2024, m, d).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(1, 1))
        .end_date(day(3, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycles().await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_period_end(day(3, 1));
    env.process_cycles().await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_status(SubscriptionStatusEnum::Completed);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    assert_eq!(
        lines_of(&invoices[0]),
        vec![line("Subscription Rate", (2024, 1, 1), (2024, 2, 1), 3500)]
    );
    assert_eq!(
        lines_of(&invoices[1]),
        vec![line("Subscription Rate", (2024, 2, 1), (2024, 3, 1), 3500)]
    );
}

/// A term ending inside the first period bills that period up to the end only.
/// Rules: R-079, R-084
#[rstest]
#[tokio::test]
async fn test_fixed_term_ending_in_the_first_period_bills_up_to_its_end(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let day = |m, d| NaiveDate::from_ymd_opt(2024, m, d).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(1, 1))
        .end_date(day(1, 20))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_period_end(day(1, 20));
    env.process_cycles().await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_status(SubscriptionStatusEnum::Completed);

    // [Jan1, Jan20) of January: 3500 × 19/31 = 2145.16 → 2145.
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    assert_eq!(
        lines_of(&invoices[0]),
        vec![line("Subscription Rate", (2024, 1, 1), (2024, 1, 20), 2145)]
    );
}

/// A term ending mid-period bills its last advance up to the end, and its completion settles the
/// usage up to the end.
/// Rules: R-011, R-016, R-079, R-084
#[tokio::test]
async fn test_fixed_term_ending_mid_period_settles_usage_up_to_its_end() {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    let day = |m, d| NaiveDate::from_ymd_opt(2024, m, d).unwrap();
    bandwidth(&usage, day(1, 1), day(2, 1), 100);
    bandwidth(&usage, day(2, 1), day(2, 15), 40);

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID) // 20.00/month + bandwidth at 0.10
        .start_date(day(1, 1))
        .end_date(day(2, 15))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycles().await;
    env.process_cycles().await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_status(SubscriptionStatusEnum::Completed);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    assert_eq!(
        lines_of(&invoices[0]),
        vec![line("Platform Fee", (2024, 1, 1), (2024, 2, 1), 2000)]
    );
    // [Feb1, Feb15) in advance: 2000 × 14/29 = 965.52 → 966; January's 100 units, 1000.
    assert_eq!(
        lines_of(&invoices[1]),
        vec![
            line("Bandwidth", (2024, 1, 1), (2024, 2, 1), 1000),
            line("Platform Fee", (2024, 2, 1), (2024, 2, 15), 966),
        ]
    );
    // The completion settles the 40 units of [Feb1, Feb15): 400.
    assert_eq!(
        lines_of(&invoices[2]),
        vec![line("Bandwidth", (2024, 2, 1), (2024, 2, 15), 400)]
    );
}

/// A fixed fee billed in arrears bills its last window up to the term's end, prorated as a
/// cancellation that day would.
/// Rules: R-002, R-016, R-084
#[tokio::test]
async fn test_fixed_term_ending_mid_period_bills_fixed_arrears_up_to_its_end() {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    let day = |m, d| NaiveDate::from_ymd_opt(2024, m, d).unwrap();
    let plan = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Term with arrears", plan)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(3000, 2),
            ),
            SeedComp::extra_recurring(
                PriceComponentId::new(),
                "Support",
                PRODUCT_RECURRING_ARREARS_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(1000, 2),
                1,
                BillingType::Arrears,
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();

    let sub_id = subscription()
        .plan_version(plan)
        .start_date(day(1, 1))
        .end_date(day(2, 15))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycles().await;
    env.process_cycles().await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_status(SubscriptionStatusEnum::Completed);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    assert_eq!(
        lines_of(&invoices[0]),
        vec![line("Platform Fee", (2024, 1, 1), (2024, 2, 1), 3000)]
    );
    // [Feb1, Feb15) in advance: 3000 × 14/29 = 1448.28 → 1448; January's support whole, 1000.
    assert_eq!(
        lines_of(&invoices[1]),
        vec![
            line("Platform Fee", (2024, 2, 1), (2024, 2, 15), 1448),
            line("Support", (2024, 1, 1), (2024, 2, 1), 1000),
        ]
    );
    // February's support for [Feb1, Feb15): 1000 × 14/29 = 482.76 → 483.
    assert_eq!(
        lines_of(&invoices[2]),
        vec![line("Support", (2024, 2, 1), (2024, 2, 15), 483)]
    );
}

/// An annual term ending mid-year bills its last year up to the end.
/// Rules: R-079, R-084
#[rstest]
#[tokio::test]
async fn test_fixed_term_ending_mid_year_bills_its_share_of_the_year(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
    let plan = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Annual term", plan)
        .components(vec![SeedComp::rate(
            PriceComponentId::new(),
            "Platform Fee",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            DieselBillingPeriodEnum::Annual,
            Decimal::new(120_000, 2),
        )])
        .seed(&mut env.conn().await)
        .await
        .unwrap();

    let sub_id = subscription()
        .plan_version(plan)
        .start_date(day(2024, 1, 1))
        .end_date(day(2025, 4, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycles().await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_period_end(day(2025, 4, 1));
    env.process_cycles().await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_status(SubscriptionStatusEnum::Completed);

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    assert_eq!(
        lines_of(&invoices[0]),
        vec![line("Platform Fee", (2024, 1, 1), (2025, 1, 1), 120_000)]
    );
    // [2025-01-01, 2025-04-01) is 31 + 28 + 31 = 90 of 2025's 365 days:
    // 120000 × 90/365 = 29589.04 → 29589.
    assert_eq!(
        lines_of(&invoices[1]),
        vec![line("Platform Fee", (2025, 1, 1), (2025, 4, 1), 29_589)]
    );
}
