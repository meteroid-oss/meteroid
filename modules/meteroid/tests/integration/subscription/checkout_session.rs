//! Checkout session flow tests.
//!
//! Tests for the full checkout session lifecycle:
//! - Create checkout session
//! - Complete checkout with payment
//! - Verify invoices (first and second billing cycles)
//!
//! These tests cover the SelfServe checkout type where both subscription
//! and invoice are created during checkout completion.

use chrono::NaiveDate;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, test_env};
use meteroid_store::domain::CreateCheckoutSession;
use meteroid_store::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutType};
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

// =============================================================================
// CHECKOUT SESSION WITH PAID TRIAL
// =============================================================================

/// Full checkout session flow with paid trial.
///
/// Flow:
/// 1. Create checkout session with 7-day paid trial
/// 2. Complete checkout with payment
/// 3. Verify first invoice (full charge, not prorated)
/// 4. Process trial end → Active
/// 5. Process renewal → second invoice
/// 6. Verify both invoices have correct dates and amounts
#[rstest]
#[tokio::test]
async fn test_checkout_session_paid_trial_invoice_dates(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

    // Create checkout session for paid trial plan
    let session = env
        .store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_PAID_TRIAL_ID, // $99/month, paid trial
            created_by: USER_ID,
            billing_start_date: Some(start_date),
            billing_day_anchor: Some(15), // Bill on the 15th
            net_terms: None,
            trial_duration_days: Some(7), // 7-day paid trial
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            components: None,
            add_ons: None,
            coupon_code: None,
            coupon_ids: vec![],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: CheckoutType::SelfServe,
            subscription_id: None,
        })
        .await
        .expect("Failed to create checkout session");

    // Complete checkout with payment
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            9900, // $99.00
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed");

    let subscription_id = match result {
        CheckoutCompletionResult::Completed {
            subscription_id,
            transaction,
        } => {
            assert!(
                transaction.is_some(),
                "Should have payment transaction for paid trial"
            );
            subscription_id
        }
        CheckoutCompletionResult::AwaitingPayment { .. } => {
            panic!("Payment should not be pending for mock provider")
        }
    };

    // === Phase 1: After checkout - TrialActive with first invoice ===
    let sub = env.get_subscription(subscription_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(false)
        .has_payment_method(true)
        .has_trial_duration(Some(7))
        .has_cycle_index(0);

    let invoices = env.get_invoices(subscription_id).await;
    invoices.assert().has_count(1);

    // First invoice: covers the first billing period (15th to 15th)
    // For paid trial, billing starts immediately but trial features apply
    let expected_period_start = start_date; // 2024-01-15
    let expected_period_end = NaiveDate::from_ymd_opt(2024, 2, 15).unwrap(); // 2024-02-15

    invoices
        .assert()
        .invoice_at(0)
        .has_total(9900) // Full $99 - NOT prorated for paid trial
        .has_invoice_date(start_date)
        .has_period(expected_period_start, expected_period_end)
        .check_prorated(false);

    // === Phase 2: Process trial end → Active ===
    env.process_cycles().await;

    let sub = env.get_subscription(subscription_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(true)
        .has_cycle_index(0); // Still cycle 0 after paid trial ends

    // Still only 1 invoice - paid trial doesn't create new invoice at trial end
    let invoices = env.get_invoices(subscription_id).await;
    invoices.assert().has_count(1);

    // === Phase 3: Process first renewal → second invoice ===
    env.process_cycles().await;

    let sub = env.get_subscription(subscription_id).await;
    sub.assert().is_active().has_cycle_index(1);

    let invoices = env.get_invoices(subscription_id).await;
    invoices.assert().has_count(2);

    // Second invoice: covers next billing period (15th to 15th)
    let expected_period_2_start = NaiveDate::from_ymd_opt(2024, 2, 15).unwrap();
    let expected_period_2_end = NaiveDate::from_ymd_opt(2024, 3, 15).unwrap();

    invoices
        .assert()
        .invoice_at(1)
        .has_total(9900)
        .has_invoice_date(expected_period_2_start)
        .has_period(expected_period_2_start, expected_period_2_end)
        .check_prorated(false);
}

// =============================================================================
// CHECKOUT SESSION WITH FREE TRIAL
// =============================================================================

/// Full checkout session flow with free trial.
///
/// Flow:
/// 1. Create checkout session with 14-day free trial
/// 2. Complete checkout (no payment, just saves payment method)
/// 3. Verify NO invoice created during trial
/// 4. Process trial end → Active with first invoice
/// 5. Process renewal → second invoice
/// 6. Verify both invoices have correct dates and amounts
#[rstest]
#[tokio::test]
async fn test_checkout_session_free_trial_invoice_dates(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    // Create checkout session for free trial plan
    let session = env
        .store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_PAID_FREE_TRIAL_ID, // $49/month, free trial
            created_by: USER_ID,
            billing_start_date: Some(start_date),
            billing_day_anchor: Some(1), // Bill on the 1st
            net_terms: None,
            trial_duration_days: Some(14), // 14-day free trial
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            components: None,
            add_ons: None,
            coupon_code: None,
            coupon_ids: vec![],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: CheckoutType::SelfServe,
            subscription_id: None,
        })
        .await
        .expect("Failed to create checkout session");

    // Complete checkout - no payment for free trial, just saves payment method
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0, // No charge for free trial
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed");

    let subscription_id = match result {
        CheckoutCompletionResult::Completed {
            subscription_id,
            transaction,
        } => {
            assert!(
                transaction.is_none(),
                "Should NOT have payment transaction for free trial"
            );
            subscription_id
        }
        CheckoutCompletionResult::AwaitingPayment { .. } => {
            panic!("Free trial should not have pending payment")
        }
    };

    // === Phase 1: After checkout - TrialActive with NO invoice ===
    let sub = env.get_subscription(subscription_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(false)
        .has_payment_method(true)
        .has_trial_duration(Some(14))
        .has_cycle_index(0);

    // No invoices during free trial
    let invoices = env.get_invoices(subscription_id).await;
    invoices.assert().assert_empty();

    // Trial period: 2024-02-01 to 2024-02-15 (14 days)
    let expected_trial_end = NaiveDate::from_ymd_opt(2024, 2, 15).unwrap();
    assert_eq!(sub.current_period_start, start_date);
    assert_eq!(sub.current_period_end, Some(expected_trial_end));

    // === Phase 2: Process trial end → Active with first invoice ===
    env.process_cycles().await;

    let sub = env.get_subscription(subscription_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(true)
        .has_cycle_index(0);

    let invoices = env.get_invoices(subscription_id).await;
    invoices.assert().has_count(1);

    // First invoice: starts after trial ends
    // Since billing_day_anchor=1 and trial ends on 2024-02-15,
    // the first billing period is PRORATED: 2024-02-15 to 2024-03-01 (~14 days)
    // This aligns future billing with the anchor date (1st of month)
    let expected_period_1_start = NaiveDate::from_ymd_opt(2024, 2, 15).unwrap();
    let expected_period_1_end = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    invoices
        .assert()
        .invoice_at(0)
        .has_invoice_date(expected_period_1_start)
        .has_period(expected_period_1_start, expected_period_1_end)
        .check_prorated(true); // First invoice IS prorated due to anchor alignment

    // Prorated amount: ~14/29 days of February * $49 ≈ $25.34
    // The exact amount depends on the proration calculation
    let first_invoice = &invoices[0];
    assert!(
        first_invoice.total < 4900,
        "First invoice should be prorated (less than $49), got {}",
        first_invoice.total
    );
    assert!(
        first_invoice.total > 2000,
        "First invoice should be reasonable (more than $20), got {}",
        first_invoice.total
    );

    let period_end_after_trial = sub.current_period_end;

    // === Phase 3: Process first renewal → second invoice (FULL billing cycle) ===
    env.process_cycles().await;

    let sub = env.get_subscription(subscription_id).await;
    sub.assert().is_active().has_cycle_index(1);

    let invoices = env.get_invoices(subscription_id).await;
    invoices.assert().has_count(2);

    // Second invoice: full month starting from anchor (1st)
    let expected_period_2_start = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
    let expected_period_2_end = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();

    invoices
        .assert()
        .invoice_at(1)
        .has_total(4900) // Full $49.00 - no proration
        .has_invoice_date(expected_period_2_start)
        .has_period(expected_period_2_start, expected_period_2_end)
        .check_prorated(false);

    // Verify period continuity
    assert_eq!(
        Some(sub.current_period_start),
        period_end_after_trial,
        "Second period should start where first ended"
    );
}

/// Free trial WITHOUT billing anchor - billing starts naturally from trial end.
///
/// This shows the difference when no billing_day_anchor is set:
/// - Trial ends on a specific date
/// - First invoice covers a full month from that date (no proration)
#[rstest]
#[tokio::test]
async fn test_checkout_session_free_trial_no_anchor(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    // Create checkout session for free trial plan WITHOUT billing anchor
    let session = env
        .store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_PAID_FREE_TRIAL_ID, // $49/month, free trial
            created_by: USER_ID,
            billing_start_date: Some(start_date),
            billing_day_anchor: None, // NO anchor - billing follows trial end naturally
            net_terms: None,
            trial_duration_days: Some(14), // 14-day free trial
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            components: None,
            add_ons: None,
            coupon_code: None,
            coupon_ids: vec![],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: CheckoutType::SelfServe,
            subscription_id: None,
        })
        .await
        .expect("Failed to create checkout session");

    // Complete checkout - no payment for free trial
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
        .expect("Checkout should succeed");

    let subscription_id = match result {
        CheckoutCompletionResult::Completed {
            subscription_id,
            transaction,
        } => {
            assert!(transaction.is_none(), "No payment for free trial");
            subscription_id
        }
        CheckoutCompletionResult::AwaitingPayment { .. } => {
            panic!("Free trial should not have pending payment")
        }
    };

    // === Phase 1: After checkout - TrialActive with NO invoice ===
    let sub = env.get_subscription(subscription_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(false)
        .has_payment_method(true)
        .has_trial_duration(Some(14))
        .has_cycle_index(0);

    let invoices = env.get_invoices(subscription_id).await;
    invoices.assert().assert_empty();

    // Trial period: 2024-02-01 to 2024-02-15
    let expected_trial_end = NaiveDate::from_ymd_opt(2024, 2, 15).unwrap();
    assert_eq!(sub.current_period_start, start_date);
    assert_eq!(sub.current_period_end, Some(expected_trial_end));

    // === Phase 2: Process trial end → Active with first invoice ===
    env.process_cycles().await;

    let sub = env.get_subscription(subscription_id).await;
    sub.assert().is_active().has_cycle_index(0);

    let invoices = env.get_invoices(subscription_id).await;
    invoices.assert().has_count(1);

    // Without billing anchor, first invoice should be a FULL month from trial end
    // Period: 2024-02-15 to 2024-03-15 (full month, no proration)
    let expected_period_1_start = NaiveDate::from_ymd_opt(2024, 2, 15).unwrap();
    let expected_period_1_end = NaiveDate::from_ymd_opt(2024, 3, 15).unwrap();

    invoices
        .assert()
        .invoice_at(0)
        .has_total(4900) // Full $49.00 - NO proration
        .has_invoice_date(expected_period_1_start)
        .has_period(expected_period_1_start, expected_period_1_end)
        .check_prorated(false);

    let period_end_after_trial = sub.current_period_end;

    // === Phase 3: Process renewal → second invoice ===
    env.process_cycles().await;

    let sub = env.get_subscription(subscription_id).await;
    sub.assert().is_active().has_cycle_index(1);

    let invoices = env.get_invoices(subscription_id).await;
    invoices.assert().has_count(2);

    // Second invoice: another full month from 15th to 15th
    let expected_period_2_start = NaiveDate::from_ymd_opt(2024, 3, 15).unwrap();
    let expected_period_2_end = NaiveDate::from_ymd_opt(2024, 4, 15).unwrap();

    invoices
        .assert()
        .invoice_at(1)
        .has_total(4900)
        .has_invoice_date(expected_period_2_start)
        .has_period(expected_period_2_start, expected_period_2_end)
        .check_prorated(false);

    // Verify period continuity
    assert_eq!(
        Some(sub.current_period_start),
        period_end_after_trial,
        "Second period should start where first ended"
    );
}

// =============================================================================
// CHECKOUT SESSION WITHOUT TRIAL
// =============================================================================

/// Full checkout session flow without trial.
///
/// Flow:
/// 1. Create checkout session without trial
/// 2. Complete checkout with payment
/// 3. Verify first invoice
/// 4. Process renewal → second invoice
/// 5. Verify both invoices have correct dates and amounts
#[rstest]
#[tokio::test]
async fn test_checkout_session_no_trial_invoice_dates(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    // Create checkout session without trial
    let session = env
        .store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID, // $35/month, no trial
            created_by: USER_ID,
            billing_start_date: Some(start_date),
            billing_day_anchor: Some(1), // Bill on the 1st
            net_terms: None,
            trial_duration_days: None, // No trial
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            components: None,
            add_ons: None,
            coupon_code: None,
            coupon_ids: vec![],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: CheckoutType::SelfServe,
            subscription_id: None,
        })
        .await
        .expect("Failed to create checkout session");

    // Complete checkout with payment
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3500, // $35.00
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout should succeed");

    let subscription_id = match result {
        CheckoutCompletionResult::Completed {
            subscription_id,
            transaction,
        } => {
            assert!(
                transaction.is_some(),
                "Should have payment transaction for paid subscription"
            );
            subscription_id
        }
        CheckoutCompletionResult::AwaitingPayment { .. } => {
            panic!("Payment should not be pending for mock provider")
        }
    };

    // === Phase 1: After checkout - Active with first invoice ===
    let sub = env.get_subscription(subscription_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_payment_method(true)
        .has_trial_duration(None)
        .has_cycle_index(0);

    let invoices = env.get_invoices(subscription_id).await;
    invoices.assert().has_count(1);

    // First invoice: covers the first billing period (1st to 1st)
    let expected_period_start = start_date; // 2024-03-01
    let expected_period_end = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap(); // 2024-04-01

    invoices
        .assert()
        .invoice_at(0)
        .has_total(3500)
        .has_invoice_date(start_date)
        .has_period(expected_period_start, expected_period_end)
        .check_prorated(false);

    // === Phase 2: Process first renewal → second invoice ===
    env.process_cycles().await;

    let sub = env.get_subscription(subscription_id).await;
    sub.assert().is_active().has_cycle_index(1);

    let invoices = env.get_invoices(subscription_id).await;
    invoices.assert().has_count(2);

    // Second invoice: covers next billing period (1st to 1st)
    let expected_period_2_start = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();
    let expected_period_2_end = NaiveDate::from_ymd_opt(2024, 5, 1).unwrap();

    invoices
        .assert()
        .invoice_at(1)
        .has_total(3500)
        .has_invoice_date(expected_period_2_start)
        .has_period(expected_period_2_start, expected_period_2_end)
        .check_prorated(false);

    // Verify subscription period dates match
    assert_eq!(
        sub.current_period_start, expected_period_2_start,
        "Subscription period start should match invoice period"
    );
    assert_eq!(
        sub.current_period_end,
        Some(expected_period_2_end),
        "Subscription period end should match invoice period"
    );
}
