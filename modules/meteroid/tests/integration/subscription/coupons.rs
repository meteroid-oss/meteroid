//! Coupon integration tests.
//!
//! Tests for:
//! - Fixed amount and percentage coupons
//! - Coupon with free and paid trials
//! - recurring_value limiting coupon to N billing cycles
//! - Plan restrictions
//! - Multi-cycle billing with coupons

use chrono::NaiveDate;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env};
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};

// =============================================================================
// BASIC COUPON TESTS
// =============================================================================

/// Fixed amount coupon applied at subscription start (no trial).
/// €10 off on a €49 plan = €39 invoice.
#[rstest]
#[tokio::test]
async fn test_fixed_coupon_no_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create a €10 fixed coupon (amount in main currency unit, not cents)
    let coupon_id = env.create_fixed_coupon("FIXED10", 10, "EUR").await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    // Should be active immediately (no trial)
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    // Invoice should show discount
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_subtotal(4900) // $49.00
        .has_discount(1000) // $10.00 discount
        .has_total(3900) // $49 - $10 = $39
        .has_coupons_count(1);
}

/// Percentage coupon applied at subscription start (no trial).
/// 20% off on a $49 plan = $39.20 invoice.
#[rstest]
#[tokio::test]
async fn test_percentage_coupon_no_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create a 20% coupon
    let coupon_id = env.create_percentage_coupon("PERCENT20", 20).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_subtotal(4900) // $49.00
        .has_discount(980) // 20% of $49 = $9.80
        .has_total(3920) // $49 - $9.80 = $39.20
        .has_coupons_count(1);
}

// =============================================================================
// COUPON + FREE TRIAL TESTS
// =============================================================================

/// Coupon with free trial: discount applies only after trial ends.
/// During trial: no invoice.
/// After trial: invoice with coupon discount.
#[rstest]
#[tokio::test]
async fn test_coupon_with_free_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let coupon_id = env.create_percentage_coupon("TRIAL20", 20).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month, free trial
        .on_start()
        .trial_days(14)
        .coupon(coupon_id)
        .create(env.services())
        .await;

    // During trial: TrialActive, no invoices
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active();

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // End trial
    env.process_cycles().await;

    // After trial: Active with discounted invoice
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_subtotal(4900)
        .has_discount(980) // 20% off
        .has_total(3920)
        .has_coupons_count(1);
}

// =============================================================================
// COUPON + PAID TRIAL TESTS
// =============================================================================

/// Coupon with paid trial: discount applies from the start.
/// Paid trials bill immediately, so coupon discount should be on the first invoice.
#[rstest]
#[tokio::test]
async fn test_coupon_with_paid_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let coupon_id = env.create_percentage_coupon("PAID20", 20).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID) // $99/month, paid trial
        .on_start()
        .trial_days(7)
        .coupon(coupon_id)
        .create(env.services())
        .await;

    // Paid trial starts as TrialActive but with an invoice
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active();

    // First invoice should already have coupon discount
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid()
        .has_subtotal(9900) // $99.00
        .has_discount(1980) // 20% off = $19.80
        .has_total(7920) // $99 - $19.80 = $79.20
        .has_coupons_count(1);
}

// =============================================================================
// RECURRING_VALUE TESTS
// =============================================================================

/// Coupon with recurring_value=2: applies for only first 2 billing cycles.
/// Cycle 0: discount applied
/// Cycle 1: discount applied
/// Cycle 2+: no discount (coupon exhausted)
///
/// NOTE: The applied_count is updated during invoice finalization, so we must
/// call run_outbox_and_orchestration() to finalize invoices between cycles.
#[rstest]
#[tokio::test]
async fn test_coupon_recurring_value_limits_cycles(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Create coupon that only applies for 2 cycles
    let coupon_id = env
        .create_limited_percentage_coupon("LIMITED2", 20, Some(2))
        .await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // €49/month
        .start_date(start_date)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    // === Cycle 0 (creation): discount applied ===
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("Cycle 0")
        .has_discount(980)
        .has_total(3920)
        .has_coupons_count(1);

    // Finalize invoice to update applied_count
    env.run_outbox_and_orchestration().await;

    // === Cycle 1 (renewal): discount still applied (applied_count=1, limit=2) ===
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("Cycle 1")
        .has_discount(980)
        .has_total(3920)
        .has_coupons_count(1);

    // Finalize invoice to update applied_count
    env.run_outbox_and_orchestration().await;

    // === Cycle 2 (renewal): coupon exhausted (applied_count=2, limit=2) ===
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    invoices
        .assert()
        .invoice_at(2)
        .with_context("Cycle 2 - coupon exhausted")
        .has_discount(0)
        .has_total(4900) // Full price
        .has_coupons_count(0);

    // === Cycle 3: still no discount ===
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(4);
    invoices
        .assert()
        .invoice_at(3)
        .with_context("Cycle 3")
        .has_discount(0)
        .has_total(4900);
}

/// Coupon with recurring_value=None (unlimited): applies forever.
#[rstest]
#[tokio::test]
async fn test_coupon_no_recurring_limit_applies_forever(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Create coupon with no recurring limit
    let coupon_id = env
        .create_limited_percentage_coupon("UNLIMITED", 20, None)
        .await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month
        .start_date(start_date)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    // Process 4 cycles
    for _ in 0..4 {
        env.process_cycles().await;
    }

    // All 5 invoices should have discount
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(5);

    for i in 0..5 {
        invoices
            .assert()
            .invoice_at(i)
            .with_context(format!("Cycle {}", i))
            .has_discount(980)
            .has_total(3920)
            .has_coupons_count(1);
    }
}

// =============================================================================
// PLAN RESTRICTION TESTS
// =============================================================================

/// Coupon restricted to specific plan: should work when plan matches.
#[rstest]
#[tokio::test]
async fn test_plan_restricted_coupon_allowed(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create coupon restricted to PLAN_PAID_FREE_TRIAL_ID
    let coupon_id = env
        .create_plan_restricted_coupon("PLANONLY", 20, vec![PLAN_PAID_FREE_TRIAL_ID])
        .await;

    // This should succeed - coupon is allowed for this plan
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .has_discount(980) // 20% of $49
        .has_coupons_count(1);
}

/// Coupon restricted to wrong plan: should fail to create subscription.
#[rstest]
#[tokio::test]
async fn test_plan_restricted_coupon_rejected(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create coupon restricted to a different plan
    let coupon_id = env
        .create_plan_restricted_coupon("WRONGPLAN", 20, vec![PLAN_PAID_TRIAL_ID]) // Wrong plan
        .await;

    // Try to use on PLAN_PAID_FREE_TRIAL_ID - should fail
    let result = env
        .services()
        .insert_subscription(
            meteroid_store::domain::CreateSubscription {
                subscription: meteroid_store::domain::SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_PAID_FREE_TRIAL_ID, // Not in allowed list
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: None,
                    activation_condition:
                        meteroid_store::domain::SubscriptionActivationCondition::OnStart,
                    trial_duration: Some(0),
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                },
                price_components: None,
                add_ons: None,
                coupons: Some(meteroid_store::domain::CreateSubscriptionCoupons {
                    coupons: vec![meteroid_store::domain::CreateSubscriptionCoupon { coupon_id }],
                }),
            },
            TENANT_ID,
        )
        .await;

    // Should fail with validation error
    assert!(
        result.is_err(),
        "Expected error when using coupon on wrong plan"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("cannot be applied") || err.contains("plan"),
        "Error should mention plan restriction: {}",
        err
    );
}

// =============================================================================
// MULTIPLE COUPONS TESTS
// =============================================================================

/// Multiple coupons can be stacked on a subscription.
#[rstest]
#[tokio::test]
async fn test_multiple_coupons_stack(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create two coupons - fixed must match plan currency (EUR)
    let coupon1_id = env.create_fixed_coupon("FIXED5", 5, "EUR").await; // €5 off
    let coupon2_id = env.create_percentage_coupon("PERCENT10", 10).await; // 10% off

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month
        .on_start()
        .no_trial()
        .coupons(vec![coupon1_id, coupon2_id])
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);

    // Both coupons should be applied
    // Order: percentage first ($49 * 10% = $4.90), then fixed ($5)
    // Or: fixed first ($5), then percentage on remainder ($44 * 10% = $4.40)
    // Either way, total discount should be significant
    let invoice = &invoices[0];
    assert_eq!(invoice.coupons.len(), 2, "Both coupons should be applied");
    assert!(
        invoice.discount > 0,
        "Total discount should be positive: {}",
        invoice.discount
    );
}

// =============================================================================
// COUPON + CHECKOUT FLOW TESTS
// =============================================================================

/// OnCheckout + coupon + free trial: coupon applies after checkout completes.
#[rstest]
#[tokio::test]
async fn test_coupon_with_checkout_and_free_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let coupon_id = env.create_percentage_coupon("CHECKOUT20", 20).await;

    let sub_id = subscription()
        .customer(CUST_UBER_ID) // Has payment method seeded
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_checkout()
        .trial_days(14)
        .auto_charge()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    // Start: TrialActive with pending checkout
    // Note: seed_payments() already created payment methods for the customer,
    // but subscription doesn't have one linked yet until checkout completes
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active().has_pending_checkout(true);

    // No invoices during trial with pending checkout
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Complete checkout (free trial = no charge)
    let mut conn = env.conn().await;
    let (transaction, _) = env
        .services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0, // Free trial - no charge
            "USD".to_string(),
            None,
        )
        .await
        .expect("Checkout should complete");

    assert!(
        transaction.is_none(),
        "No payment during free trial checkout"
    );

    // After checkout: still in trial but payment method attached
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(false)
        .has_resolved_payment_method(&env, true)
        .await;

    // End trial → TrialExpired (OnCheckout requires payment)
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("TrialExpired awaiting payment")
        .is_trial_expired()
        .has_pending_checkout(false)
        .has_resolved_payment_method(&env, true)
        .await;

    // Invoice created with coupon discount
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .has_discount(980)
        .has_total(3920)
        .has_coupons_count(1);

    // Payment settles → Active
    env.run_outbox_and_orchestration().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Active after payment")
        .is_active();
}

// =============================================================================
// COUPON EXPIRATION TESTS
// =============================================================================

/// Coupon with expiry date: cannot be applied after expiration.
#[rstest]
#[tokio::test]
async fn test_expired_coupon_rejected(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create a coupon that already expired
    let coupon_id = env
        .create_coupon(CouponNew {
            code: "EXPIRED".to_string(),
            description: "Already expired coupon".to_string(),
            tenant_id: TENANT_ID,
            discount: CouponDiscount::Percentage(Decimal::from(20)),
            expires_at: Some(
                chrono::NaiveDateTime::parse_from_str("2020-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")
                    .unwrap(),
            ),
            redemption_limit: None,
            recurring_value: None,
            reusable: false,
            plan_ids: vec![],
        })
        .await;

    // Try to use expired coupon - should fail
    let result = env
        .services()
        .insert_subscription(
            meteroid_store::domain::CreateSubscription {
                subscription: meteroid_store::domain::SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_PAID_FREE_TRIAL_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: None,
                    activation_condition:
                        meteroid_store::domain::SubscriptionActivationCondition::OnStart,
                    trial_duration: Some(0),
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                },
                price_components: None,
                add_ons: None,
                coupons: Some(meteroid_store::domain::CreateSubscriptionCoupons {
                    coupons: vec![meteroid_store::domain::CreateSubscriptionCoupon { coupon_id }],
                }),
            },
            TENANT_ID,
        )
        .await;

    // Should fail with validation error about expiration
    assert!(result.is_err(), "Expected error when using expired coupon");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("expired") || err.contains("Expired"),
        "Error should mention expiration: {}",
        err
    );
}

// =============================================================================
// DISABLED & ARCHIVED COUPON TESTS
// =============================================================================

/// Disabled coupon: cannot be applied to new subscriptions.
#[rstest]
#[tokio::test]
async fn test_disabled_coupon_rejected(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create and then disable a coupon
    let coupon_id = env.create_percentage_coupon("DISABLED", 20).await;
    env.disable_coupon(coupon_id).await;

    // Try to use disabled coupon - should fail
    let result = env
        .services()
        .insert_subscription(
            meteroid_store::domain::CreateSubscription {
                subscription: meteroid_store::domain::SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_PAID_FREE_TRIAL_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: None,
                    activation_condition:
                        meteroid_store::domain::SubscriptionActivationCondition::OnStart,
                    trial_duration: Some(0),
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                },
                price_components: None,
                add_ons: None,
                coupons: Some(meteroid_store::domain::CreateSubscriptionCoupons {
                    coupons: vec![meteroid_store::domain::CreateSubscriptionCoupon { coupon_id }],
                }),
            },
            TENANT_ID,
        )
        .await;

    assert!(result.is_err(), "Expected error when using disabled coupon");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("disabled"),
        "Error should mention disabled: {}",
        err
    );
}

/// Archived coupon: cannot be applied to new subscriptions.
#[rstest]
#[tokio::test]
async fn test_archived_coupon_rejected(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create and then archive a coupon
    let coupon_id = env.create_percentage_coupon("ARCHIVED", 20).await;
    env.archive_coupon(coupon_id).await;

    // Try to use archived coupon - should fail
    let result = env
        .services()
        .insert_subscription(
            meteroid_store::domain::CreateSubscription {
                subscription: meteroid_store::domain::SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_PAID_FREE_TRIAL_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: None,
                    activation_condition:
                        meteroid_store::domain::SubscriptionActivationCondition::OnStart,
                    trial_duration: Some(0),
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                },
                price_components: None,
                add_ons: None,
                coupons: Some(meteroid_store::domain::CreateSubscriptionCoupons {
                    coupons: vec![meteroid_store::domain::CreateSubscriptionCoupon { coupon_id }],
                }),
            },
            TENANT_ID,
        )
        .await;

    assert!(result.is_err(), "Expected error when using archived coupon");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("archived"),
        "Error should mention archived: {}",
        err
    );
}

// =============================================================================
// REUSABLE FLAG TESTS
// =============================================================================

/// Non-reusable coupon: same customer cannot use it twice.
#[rstest]
#[tokio::test]
async fn test_non_reusable_coupon_same_customer_rejected(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create a non-reusable coupon (default)
    let coupon_id = env.create_percentage_coupon("ONETIME", 20).await;

    // First subscription with this coupon - should succeed
    let _sub1 = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    // Second subscription with same coupon and same customer - should fail
    let result = env
        .services()
        .insert_subscription(
            meteroid_store::domain::CreateSubscription {
                subscription: meteroid_store::domain::SubscriptionNew {
                    customer_id: CUST_UBER_ID,                   // Same customer
                    plan_version_id: PLAN_VERSION_PAID_TRIAL_ID, // Different plan
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
                    end_date: None,
                    billing_start_date: None,
                    activation_condition:
                        meteroid_store::domain::SubscriptionActivationCondition::OnStart,
                    trial_duration: Some(0),
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                },
                price_components: None,
                add_ons: None,
                coupons: Some(meteroid_store::domain::CreateSubscriptionCoupons {
                    coupons: vec![meteroid_store::domain::CreateSubscriptionCoupon { coupon_id }],
                }),
            },
            TENANT_ID,
        )
        .await;

    assert!(
        result.is_err(),
        "Expected error when same customer reuses non-reusable coupon"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("reusable") || err.contains("already been used"),
        "Error should mention reusability: {}",
        err
    );
}

/// Reusable coupon: same customer can use it multiple times.
#[rstest]
#[tokio::test]
async fn test_reusable_coupon_same_customer_allowed(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create a reusable coupon
    let coupon_id = env.create_reusable_coupon("REUSABLE", 20).await;

    // First subscription
    let sub1 = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    let sub1_row = env.get_subscription(sub1).await;
    sub1_row.assert().is_active();

    // Second subscription with same coupon and same customer - should succeed
    let sub2 = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    let sub2_row = env.get_subscription(sub2).await;
    sub2_row.assert().is_active();

    // Both subscriptions should have the coupon applied
    let invoices1 = env.get_invoices(sub1).await;
    let invoices2 = env.get_invoices(sub2).await;

    assert_eq!(invoices1.len(), 1);
    assert_eq!(invoices2.len(), 1);
    assert!(
        invoices1[0].discount > 0,
        "First subscription should have discount"
    );
    assert!(
        invoices2[0].discount > 0,
        "Second subscription should have discount"
    );
}

/// Non-reusable coupon: different customers can use it.
#[rstest]
#[tokio::test]
async fn test_non_reusable_coupon_different_customers_allowed(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create a non-reusable coupon
    let coupon_id = env.create_percentage_coupon("DIFFCUST", 20).await;

    // First customer uses coupon
    let sub1 = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    // Different customer uses same coupon - should succeed
    let sub2 = subscription()
        .customer(CUST_SPOTIFY_ID) // Different customer
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    let sub1_row = env.get_subscription(sub1).await;
    let sub2_row = env.get_subscription(sub2).await;

    sub1_row.assert().is_active();
    sub2_row.assert().is_active();
}

// =============================================================================
// REDEMPTION LIMIT TESTS
// =============================================================================

/// Coupon with redemption_limit=2: can only be applied to 2 subscriptions total.
#[rstest]
#[tokio::test]
async fn test_redemption_limit_reached(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create coupon with limit of 2 redemptions
    let coupon_id = env.create_limited_redemption_coupon("LIMIT2", 20, 2).await;

    // First subscription - should succeed
    let _sub1 = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    // Second subscription - should succeed (limit not yet reached)
    let _sub2 = subscription()
        .customer(CUST_SPOTIFY_ID)
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    // Third subscription - should fail (limit reached)
    let result = env
        .services()
        .insert_subscription(
            meteroid_store::domain::CreateSubscription {
                subscription: meteroid_store::domain::SubscriptionNew {
                    customer_id: CUST_COMODO_ID,
                    plan_version_id: PLAN_VERSION_PAID_FREE_TRIAL_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: None,
                    activation_condition:
                        meteroid_store::domain::SubscriptionActivationCondition::OnStart,
                    trial_duration: Some(0),
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                },
                price_components: None,
                add_ons: None,
                coupons: Some(meteroid_store::domain::CreateSubscriptionCoupons {
                    coupons: vec![meteroid_store::domain::CreateSubscriptionCoupon { coupon_id }],
                }),
            },
            TENANT_ID,
        )
        .await;

    assert!(
        result.is_err(),
        "Expected error when redemption limit is reached"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("redemption") || err.contains("maximum"),
        "Error should mention redemption limit: {}",
        err
    );
}

// =============================================================================
// CURRENCY MISMATCH TESTS
// =============================================================================

/// Fixed coupon with wrong currency: cannot be applied.
#[rstest]
#[tokio::test]
async fn test_currency_mismatch_rejected(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create a USD fixed coupon
    let coupon_id = env.create_fixed_coupon("USD10", 10, "USD").await;

    // Try to apply to EUR plan - should fail
    // Note: PLAN_VERSION_PAID_FREE_TRIAL_ID uses EUR currency
    let result = env
        .services()
        .insert_subscription(
            meteroid_store::domain::CreateSubscription {
                subscription: meteroid_store::domain::SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_PAID_FREE_TRIAL_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: None,
                    activation_condition:
                        meteroid_store::domain::SubscriptionActivationCondition::OnStart,
                    trial_duration: Some(0),
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                },
                price_components: None,
                add_ons: None,
                coupons: Some(meteroid_store::domain::CreateSubscriptionCoupons {
                    coupons: vec![meteroid_store::domain::CreateSubscriptionCoupon { coupon_id }],
                }),
            },
            TENANT_ID,
        )
        .await;

    assert!(
        result.is_err(),
        "Expected error when coupon currency doesn't match subscription"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("currency"),
        "Error should mention currency mismatch: {}",
        err
    );
}

// =============================================================================
// ADVANCED PLAN RESTRICTION TESTS
// =============================================================================

/// Coupon restricted to multiple plans: should work on any of the allowed plans.
#[rstest]
#[tokio::test]
async fn test_plan_restricted_coupon_multiple_plans(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create coupon restricted to TWO plans
    let coupon_id = env
        .create_plan_restricted_coupon(
            "MULTIPLAN",
            20,
            vec![PLAN_PAID_FREE_TRIAL_ID, PLAN_PAID_TRIAL_ID],
        )
        .await;

    // Should work on first allowed plan
    let sub1 = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    let sub1_row = env.get_subscription(sub1).await;
    sub1_row.assert().is_active();

    // Should work on second allowed plan (different customer to avoid reusable check)
    let sub2 = subscription()
        .customer(CUST_SPOTIFY_ID)
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    let sub2_row = env.get_subscription(sub2).await;
    sub2_row.assert().is_active();
}

/// Coupon with empty plan_ids should work on ANY plan (no restriction).
#[rstest]
#[tokio::test]
async fn test_empty_plan_restriction_allows_all_plans(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create coupon with empty plan_ids (applies to all plans)
    let coupon_id = env.create_percentage_coupon("ALLPLANS", 20).await;

    // Should work on PLAN_PAID_FREE_TRIAL
    let sub1 = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    let sub1_row = env.get_subscription(sub1).await;
    sub1_row.assert().is_active();

    // Should also work on PLAN_PAID_TRIAL (different customer, different plan)
    let sub2 = subscription()
        .customer(CUST_SPOTIFY_ID)
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    let sub2_row = env.get_subscription(sub2).await;
    sub2_row.assert().is_active();

    // Both subscriptions on different plans should have coupon applied,
    // demonstrating that empty plan_ids means "all plans"
    let invoices1 = env.get_invoices(sub1).await;
    let invoices2 = env.get_invoices(sub2).await;

    assert_eq!(
        invoices1[0].coupons.len(),
        1,
        "Coupon should apply to first plan"
    );
    assert_eq!(
        invoices2[0].coupons.len(),
        1,
        "Coupon should apply to second plan"
    );
}

// =============================================================================
// MULTIPLE COUPONS WITH PLAN RESTRICTIONS
// =============================================================================

/// Multiple coupons where some have plan restrictions.
#[rstest]
#[tokio::test]
async fn test_multiple_coupons_with_plan_restrictions(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create one coupon restricted to the plan
    let restricted_coupon = env
        .create_plan_restricted_coupon("RESTRICTED", 10, vec![PLAN_PAID_FREE_TRIAL_ID])
        .await;

    // Create one coupon with no restrictions
    let unrestricted_coupon = env.create_percentage_coupon("UNRESTRICTED", 5).await;

    // Both should apply to PLAN_PAID_FREE_TRIAL
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupons(vec![restricted_coupon, unrestricted_coupon])
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(
        invoices[0].coupons.len(),
        2,
        "Both coupons should be applied"
    );
}

/// Multiple coupons where one fails plan restriction should fail entire subscription.
#[rstest]
#[tokio::test]
async fn test_multiple_coupons_one_fails_plan_restriction(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create one coupon that WILL work
    let working_coupon = env.create_percentage_coupon("WORKS", 10).await;

    // Create one coupon restricted to a DIFFERENT plan
    let failing_coupon = env
        .create_plan_restricted_coupon("FAILS", 5, vec![PLAN_PAID_TRIAL_ID]) // Wrong plan!
        .await;

    // Try to apply both - should fail because one doesn't match
    let result = env
        .services()
        .insert_subscription(
            meteroid_store::domain::CreateSubscription {
                subscription: meteroid_store::domain::SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_PAID_FREE_TRIAL_ID, // Not PLAN_PAID_TRIAL_ID!
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: None,
                    activation_condition:
                        meteroid_store::domain::SubscriptionActivationCondition::OnStart,
                    trial_duration: Some(0),
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                },
                price_components: None,
                add_ons: None,
                coupons: Some(meteroid_store::domain::CreateSubscriptionCoupons {
                    coupons: vec![
                        meteroid_store::domain::CreateSubscriptionCoupon {
                            coupon_id: working_coupon,
                        },
                        meteroid_store::domain::CreateSubscriptionCoupon {
                            coupon_id: failing_coupon,
                        },
                    ],
                }),
            },
            TENANT_ID,
        )
        .await;

    assert!(
        result.is_err(),
        "Expected error when one coupon fails plan restriction"
    );
}

// =============================================================================
// DUPLICATE COUPON TESTS
// =============================================================================

/// Same coupon applied twice in one request should be deduplicated.
#[rstest]
#[tokio::test]
async fn test_duplicate_coupon_deduplicated(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let coupon_id = env.create_percentage_coupon("DEDUP", 20).await;

    // Apply same coupon twice
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupons(vec![coupon_id, coupon_id]) // Duplicate!
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    // Should only have 1 coupon applied (deduplicated)
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(
        invoices[0].coupons.len(),
        1,
        "Duplicate coupon should be deduplicated"
    );
}

// =============================================================================
// 100% DISCOUNT COUPON TESTS (ZERO AMOUNT CHECKOUT)
// =============================================================================

/// OnCheckout + 100% coupon: checkout with zero amount should succeed.
/// Invoice created with 0 total, subscription activated, no payment transaction.
#[rstest]
#[tokio::test]
async fn test_100_percent_coupon_checkout_succeeds(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    // Create a 100% discount coupon
    let coupon_id = env.create_percentage_coupon("FULL100", 100).await;

    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month
        .on_checkout()
        .no_trial()
        .auto_charge()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    // Start: PendingActivation with pending checkout
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_pending_activation()
        .has_pending_checkout(true);

    // No invoices yet
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Complete checkout with zero amount (100% discount)
    let mut conn = env.conn().await;
    let (transaction, is_pending) = env
        .services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0, // Zero amount due to 100% coupon
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout with 100% coupon should succeed");

    // No payment transaction for zero amount
    assert!(
        transaction.is_none(),
        "No payment transaction for zero amount checkout"
    );
    assert!(!is_pending, "Should not be pending");

    // After checkout: Active with invoice
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .has_resolved_payment_method(&env, true)
        .await;

    // Invoice should exist with 0 total
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .has_subtotal(4900) // $49.00 base
        .has_discount(4900) // 100% discount
        .has_total(0) // $0 total
        .has_coupons_count(1);
}

/// OnCheckout + 100% coupon + paid trial: checkout with zero amount should work.
/// Even with paid trial, if coupon makes it free, no payment needed.
#[rstest]
#[tokio::test]
async fn test_100_percent_coupon_with_paid_trial_checkout(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    // Create a 100% discount coupon
    let coupon_id = env.create_percentage_coupon("PAIDTRIAL100", 100).await;

    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID) // $99/month, paid trial
        .on_checkout()
        .trial_days(7)
        .auto_charge()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    // Start: PendingActivation
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_pending_activation()
        .has_pending_checkout(true);

    // Complete checkout with zero amount
    let mut conn = env.conn().await;
    let (transaction, is_pending) = env
        .services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0, // Zero amount due to 100% coupon
            "EUR".to_string(),
            None,
        )
        .await
        .expect("Checkout with 100% coupon should succeed");

    assert!(transaction.is_none(), "No payment for zero amount");
    assert!(!is_pending);

    // After checkout: TrialActive (paid trial goes to TrialActive)
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(false)
        .has_trial_duration(Some(7))
        .has_resolved_payment_method(&env, true)
        .await;

    // Invoice with 0 total
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .has_subtotal(9900) // $99.00 base
        .has_discount(9900) // 100% discount
        .has_total(0)
        .has_coupons_count(1);
}

/// 100% coupon limited to 1 cycle: first invoice free, second invoice full price.
#[rstest]
#[tokio::test]
async fn test_100_percent_coupon_limited_cycles(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Create 100% coupon limited to 1 cycle
    let coupon_id = env
        .create_limited_percentage_coupon("ONEFREE", 100, Some(1))
        .await;

    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month
        .start_date(start_date)
        .on_checkout()
        .no_trial()
        .auto_charge()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    // Complete checkout with zero amount
    let mut conn = env.conn().await;
    let (transaction, _) = env
        .services()
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

    assert!(transaction.is_none(), "No payment for zero amount");

    // First invoice: 100% discount
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("Invoice 1: 100% discount")
        .has_total(0)
        .has_coupons_count(1);

    // Finalize to update coupon applied_count
    env.run_outbox_and_orchestration().await;

    // Process renewal - coupon exhausted
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("Invoice 2: coupon exhausted")
        .has_total(4900) // Full price
        .has_coupons_count(0);
}

/// OnCheckout + 100% coupon + free trial: coupon attaches during checkout.
/// During free trial, no invoice. After trial ends, invoice with 100% discount.
#[rstest]
#[tokio::test]
async fn test_100_percent_coupon_with_free_trial_checkout(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    // Create a 100% discount coupon
    let coupon_id = env.create_percentage_coupon("FREETRIAL100", 100).await;

    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month, free trial plan
        .on_checkout()
        .trial_days(14) // 14-day free trial
        .auto_charge()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    // Start: TrialActive with pending checkout
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active().has_pending_checkout(true);

    // No invoices during trial
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Complete checkout (free trial = no charge, but coupon attached)
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
        .expect("Checkout should complete");

    assert!(
        transaction.is_none(),
        "No payment during free trial checkout"
    );

    // After checkout: still in trial, payment method attached
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_trial_active()
        .has_pending_checkout(false)
        .has_trial_duration(Some(14))
        .has_resolved_payment_method(&env, true)
        .await;

    // Still no invoices during trial
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Process trial end → TrialExpired (OnCheckout requires payment)
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("TrialExpired awaiting payment")
        .is_trial_expired()
        .has_pending_checkout(false)
        .has_resolved_payment_method(&env, true)
        .await;

    // Invoice should have 100% discount applied
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("Invoice after trial with 100% coupon")
        .has_subtotal(4900) // $49.00 base
        .has_discount(4900) // 100% discount
        .has_total(0) // $0 total
        .has_coupons_count(1);

    // Payment settles (no-op for $0 invoice) → Active
    env.run_outbox_and_orchestration().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("Active after payment")
        .is_active();
}

/// Multiple coupons are applied sequentially (each on remaining amount).
/// Two 60% coupons: 60% of $49 = $29.40 off → $19.60, then 60% of $19.60 = $11.76 off → $7.84.
#[rstest]
#[tokio::test]
async fn test_multiple_coupons_sequential_application(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create two 60% coupons
    let coupon1_id = env.create_percentage_coupon("SIXTY1", 60).await;
    let coupon2_id = env.create_percentage_coupon("SIXTY2", 60).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month
        .on_start()
        .no_trial()
        .coupons(vec![coupon1_id, coupon2_id])
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);

    // Both coupons applied sequentially
    let invoice = &invoices[0];
    assert_eq!(invoice.coupons.len(), 2, "Both coupons should be applied");

    // Sequential application: 60% of 4900 = 2940, remaining 1960
    // Then 60% of 1960 = 1176, remaining 784
    // Total discount = 2940 + 1176 = 4116
    invoices
        .assert()
        .invoice_at(0)
        .has_subtotal(4900) // $49.00 base
        .has_discount(4116) // Sequential: 2940 + 1176
        .has_total(784); // $7.84 remaining
}

/// Multiple 100% coupons: total discount capped at subtotal.
/// Two 100% coupons should result in 100% total discount, not 200%.
#[rstest]
#[tokio::test]
async fn test_multiple_100_percent_coupons_capped(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create two coupons, going over 100% total
    let coupon1_id = env.create_percentage_coupon("P1", 90).await;
    let coupon2_id = env.create_fixed_coupon("FULL2", 10, "EUR").await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month
        .on_start()
        .no_trial()
        .coupons(vec![coupon1_id, coupon2_id])
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);

    let invoice = &invoices[0];

    println!("coupons applied: {:#?}", invoice.coupons);
    assert_eq!(invoice.coupons.len(), 2, "Both coupons should be applied");

    // First 100% coupon takes full discount, second applies to $0 remaining
    // Total discount should equal subtotal (capped)
    assert!(
        invoice.discount <= invoice.subtotal,
        "Discount ({}) should not exceed subtotal ({})",
        invoice.discount,
        invoice.subtotal
    );

    invoices
        .assert()
        .invoice_at(0)
        .has_subtotal(4900) // $49.00 base
        .has_discount(4900) // Capped at subtotal
        .has_total(0); // $0 - fully discounted
}

/// Fixed coupon exceeding invoice amount: discount capped at subtotal.
/// A $100 fixed coupon on a $49 plan should only discount $49.
#[rstest]
#[tokio::test]
async fn test_fixed_coupon_exceeds_subtotal_capped(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create a €100 fixed coupon (exceeds plan price of €49)
    let coupon_id = env.create_fixed_coupon("BIGFIXED", 100, "EUR").await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/month
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);

    let invoice = &invoices[0];

    // Discount should be capped at subtotal, not the full $100
    assert!(
        invoice.discount <= invoice.subtotal,
        "Discount ({}) should not exceed subtotal ({})",
        invoice.discount,
        invoice.subtotal
    );

    assert!(
        invoice.total >= 0,
        "Total ({}) should not be negative",
        invoice.total
    );

    invoices
        .assert()
        .invoice_at(0)
        .has_subtotal(4900) // $49.00 base
        .has_discount(4900) // Capped at subtotal (not $100)
        .has_total(0); // $0 - fully discounted
}
