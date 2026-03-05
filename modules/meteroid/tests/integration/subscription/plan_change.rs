//! Plan change integration tests.
//!
//! Tests for scheduling, previewing, canceling, and executing plan changes.
//! Uses product-backed plans (Starter & Pro) where components match by product_id.

use chrono::{NaiveDate, NaiveTime};
use common_domain::ids::*;
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use rstest::rstest;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::uuid;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{
    InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env, test_env_with_usage,
};
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageClient, MockUsageDataParams, UsageData,
};
use meteroid_store::domain::subscription_components::{
    ComponentParameterization, ComponentParameters,
};
use meteroid_store::domain::{BillingType, Period, SlotUpgradeBillingMode};
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;
use rust_decimal::Decimal;

// =============================================================================
// PREVIEW
// =============================================================================

/// Preview returns matched, added, and removed components.
/// Starter→Pro: both share Platform Fee and Seats products, so all matched, none added/removed.
#[rstest]
#[tokio::test]
async fn test_preview_plan_change(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let result = env
        .services()
        .preview_plan_change(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![], None)
        .await
        .expect("preview_plan_change failed");

    let preview = &result.preview;

    // Both plans share the same 2 products → 2 matched, 0 added, 0 removed
    assert_eq!(preview.matched.len(), 2, "expected 2 matched components");
    assert!(preview.added.is_empty(), "expected no added components");
    assert!(preview.removed.is_empty(), "expected no removed components");

    // Effective date should be the subscription's current_period_end
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(
        preview.effective_date,
        sub.current_period_end.unwrap(),
        "effective_date should match current_period_end"
    );
}

// =============================================================================
// SCHEDULE & CANCEL
// =============================================================================

/// Schedule creates a pending event at period end.
#[rstest]
#[tokio::test]
async fn test_schedule_plan_change(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let event = env
        .services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![])
        .await
        .expect("schedule_plan_change failed");

    assert_eq!(event.subscription_id, sub_id);

    let sub = env.get_subscription(sub_id).await;
    let expected_time = sub
        .current_period_end
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    assert_eq!(event.scheduled_time, expected_time);
}

/// Cancel removes the pending event.
#[rstest]
#[tokio::test]
async fn test_cancel_plan_change(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Schedule then cancel
    env.services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![])
        .await
        .expect("schedule_plan_change failed");

    env.services()
        .cancel_plan_change(sub_id, TENANT_ID)
        .await
        .expect("cancel_plan_change failed");

    // Scheduling again should succeed (no duplicate)
    env.services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![])
        .await
        .expect("re-scheduling after cancel should succeed");
}

// =============================================================================
// EXECUTION
// =============================================================================

/// Full lifecycle: Starter invoice → schedule change → process_cycles → Pro invoice.
/// Validates that the first invoice is at Starter prices and the second at Pro prices.
/// Also verifies component details (price_component_id, name) after plan change.
///
/// Starter: Platform Fee €29 + Seats €10×1 = €39/mo (3900 cents)
/// Pro:     Platform Fee €99 + Seats €25×1 = €124/mo (12400 cents)
#[rstest]
#[tokio::test]
async fn test_plan_change_executes_at_period_end(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // --- Cycle 0: Starter ---

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(0)
        .has_period_start(start_date)
        .has_period_end(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap());

    // First invoice: Starter prices
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("cycle 0 - Starter invoice")
        .is_finalized_unpaid()
        .has_total(3900)
        .has_period(
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
        );

    // --- Schedule plan change to Pro ---

    env.services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![])
        .await
        .expect("schedule_plan_change failed");

    // --- Cycle 1: process_cycles applies plan change AND renews in one pass ---
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(
        sub.plan_version_id, PLAN_VERSION_PRO_ID,
        "plan change should have been applied"
    );
    sub.assert()
        .is_active()
        .has_cycle_index(1)
        .has_period_start(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
        .has_period_end(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap());

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("cycle 1 - first Pro invoice")
        .is_finalized_unpaid()
        .has_total(12400)
        .has_period(
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        );

    // --- Cycle 2: verify continued Pro pricing ---
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    invoices
        .assert()
        .invoice_at(2)
        .with_context("cycle 2 - continued Pro pricing")
        .is_finalized_unpaid()
        .has_total(12400)
        .has_period(
            NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 4, 1).unwrap(),
        );

    // --- Verify component details point to Pro plan ---
    let components = env.get_subscription_components(sub_id).await;
    assert_eq!(components.len(), 2, "should have 2 components on Pro plan");

    // Verify each component references the correct Pro price_component_id
    let comp_ids: Vec<_> = components
        .iter()
        .filter_map(|c| c.price_component_id)
        .collect();
    assert!(
        comp_ids.contains(&COMP_PRO_PLATFORM_FEE_ID),
        "should have Pro Platform Fee component (got {:?})",
        comp_ids
    );
    assert!(
        comp_ids.contains(&COMP_PRO_SEATS_ID),
        "should have Pro Seats component (got {:?})",
        comp_ids
    );

    for comp in &components {
        assert!(
            comp.product_id.is_some(),
            "component '{}' should have product_id set",
            comp.name
        );
    }
}

// =============================================================================
// REJECTION TESTS
// =============================================================================

/// Cannot schedule change to a draft plan version.
#[rstest]
#[tokio::test]
async fn test_plan_change_rejects_draft_target(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let result = env
        .services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_PRO_DRAFT_ID, vec![])
        .await;

    assert!(result.is_err(), "should reject draft plan version");
}

/// Cannot change to plan with different currency.
#[rstest]
#[tokio::test]
async fn test_plan_change_rejects_currency_mismatch(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Sub is on EUR plan (Starter)
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Try to change to USD plan
    let result = env
        .services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_USD_ID, vec![])
        .await;

    assert!(result.is_err(), "should reject currency mismatch");
}

/// Scheduling a second plan change replaces (cancels) the first one.
#[rstest]
#[tokio::test]
async fn test_plan_change_replaces_existing(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Start on LeetCode so we can schedule to both Starter and Pro
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // First schedule: LeetCode → Starter
    let first_event = env
        .services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_STARTER_ID, vec![])
        .await
        .expect("first schedule should succeed");

    // Second schedule: LeetCode → Pro (replaces Starter change)
    let second_event = env
        .services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![])
        .await
        .expect("second schedule should succeed (replaces first)");

    assert_ne!(first_event.id, second_event.id, "should be a new event");

    // Only one pending plan change should exist
    let mut conn = env.conn().await;
    let pending =
        diesel_models::scheduled_events::ScheduledEventRow::get_pending_events_for_subscription(
            &mut conn, sub_id, &TENANT_ID,
        )
        .await
        .expect("query pending events");

    assert_eq!(pending.len(), 1, "should have exactly one pending event");
    assert_eq!(
        pending[0].id, second_event.id,
        "pending event should be the second one"
    );
}

/// Cannot schedule on a pending subscription.
#[rstest]
#[tokio::test]
async fn test_plan_change_rejects_inactive_subscription(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Create a PendingActivation subscription (Manual, not yet activated)
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .manual()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_pending_activation();

    let result = env
        .services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![])
        .await;

    assert!(
        result.is_err(),
        "should reject plan change on pending subscription"
    );
}

/// Cannot schedule a change to the current plan version.
#[rstest]
#[tokio::test]
async fn test_plan_change_rejects_same_plan_version(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Schedule change to the same plan version
    let result = env
        .services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_STARTER_ID, vec![])
        .await;

    assert!(
        result.is_err(),
        "should reject plan change to current plan version"
    );

    // Also test immediate path
    let result = env
        .services()
        .apply_plan_change_immediate(sub_id, TENANT_ID, PLAN_VERSION_STARTER_ID, vec![])
        .await;

    assert!(
        result.is_err(),
        "should reject immediate change to current plan version"
    );
}

// =============================================================================
// IMMEDIATE PLAN CHANGE
// =============================================================================

/// Immediate mid-period upgrade: Starter→Pro with deterministic proration.
///
/// Fixed dates: start Jan 1, period [Jan 1, Feb 1] = 31 days, change on Jan 16 → 16 days remaining.
/// factor = 16/31
///
/// Starter: Platform Fee €29 + Seats €10×1 = €39/mo (3900 cents)
/// Pro:     Platform Fee €99 + Seats €25×1 = €124/mo (12400 cents)
///
/// Expected adjustment (per-component rounding):
///   Credit Platform: -(2900 × 16/31).round() = -1497
///   Credit Seats:    -(1000 × 16/31).round() = -516
///   Charge Platform:  (9900 × 16/31).round() = 5110
///   Charge Seats:     (2500 × 16/31).round() = 1290
///   Net: -1497 + -516 + 5110 + 1290 = 4387
#[rstest]
#[tokio::test]
async fn test_immediate_plan_change_upgrade(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(0)
        .has_period_start(start_date)
        .has_period_end(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap());

    // Apply immediate upgrade at Jan 16
    let result = env
        .services()
        .apply_plan_change_immediate_at(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![], change_date)
        .await
        .expect("apply_plan_change_immediate_at failed");

    assert!(
        result.adjustment_invoice_id.is_some(),
        "should create adjustment invoice for non-zero proration"
    );

    // Subscription should now be on Pro plan
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(
        sub.plan_version_id, PLAN_VERSION_PRO_ID,
        "plan_version_id should be Pro"
    );
    sub.assert().is_active().has_cycle_index(0);

    // Components should be Pro components with correct IDs
    let components = env.get_subscription_components(sub_id).await;
    assert_eq!(components.len(), 2, "should have 2 Pro components");
    let comp_ids: Vec<_> = components
        .iter()
        .filter_map(|c| c.price_component_id)
        .collect();
    assert!(
        comp_ids.contains(&COMP_PRO_PLATFORM_FEE_ID),
        "should have Pro Platform Fee component"
    );
    assert!(
        comp_ids.contains(&COMP_PRO_SEATS_ID),
        "should have Pro Seats component"
    );

    // Verify temporal bounds on all components (active + closed)
    let all_components = env
        .get_all_subscription_components(
            sub_id,
            start_date,
            change_date + chrono::Duration::days(1),
        )
        .await;
    assert_eq!(
        all_components.len(),
        4,
        "should have 4 total components (2 closed Starter + 2 active Pro)"
    );

    let closed: Vec<_> = all_components
        .iter()
        .filter(|c| c.effective_to.is_some())
        .collect();
    for c in &closed {
        assert_eq!(
            c.effective_to,
            Some(change_date),
            "closed component '{}' should have effective_to = change_date",
            c.name
        );
    }

    let active: Vec<_> = all_components
        .iter()
        .filter(|c| c.effective_to.is_none())
        .collect();
    for c in &active {
        assert_eq!(
            c.effective_from, change_date,
            "new component '{}' should have effective_from = change_date",
            c.name
        );
    }

    // Should have: initial Starter invoice + adjustment invoice
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);

    // Adjustment invoice should be finalized and prorated
    invoices
        .assert()
        .invoice_at(1)
        .with_context("adjustment invoice")
        .has_status(meteroid_store::domain::enums::InvoiceStatusEnum::Finalized)
        .check_prorated(true);

    // Hardcoded expected proration: 16 days remaining out of 31-day period (Jan 1→Feb 1)
    // Credit Platform: -(2900 × 16/31).round() = -1497
    // Credit Seats:    -(1000 × 16/31).round() = -516
    // Charge Platform:  (9900 × 16/31).round() = 5110
    // Charge Seats:     (2500 × 16/31).round() = 1290
    // Net: 4387
    invoices
        .assert()
        .invoice_at(1)
        .with_context("upgrade adjustment")
        .has_total(4387)
        .has_amount_due(4387)
        .has_payment_status(meteroid_store::domain::InvoicePaymentStatus::Unpaid);

    // Customer balance should be unchanged (no credit for upgrades)
    let sub = env.get_subscription(sub_id).await;
    let customer = env.get_customer(sub.customer_id).await;
    assert_eq!(
        customer.balance_value_cents, 0,
        "customer balance should remain 0 after upgrade"
    );
}

/// Immediate downgrade: Pro→Starter produces a negative adjustment (credit).
///
/// Fixed dates: start Jan 1, period [Jan 1, Feb 1] = 31 days, change on Jan 16 → 16 days remaining.
/// factor = 16/31
///
/// Pro:     Platform Fee €99 (9900) + Seats €25×1 (2500) = €124/mo (12400 cents)
/// Starter: Platform Fee €29 (2900) + Seats €10×1 (1000) = €39/mo (3900 cents)
///
/// Expected adjustment:
///   Credit Platform: -(9900 × 16/31).round() = -5110
///   Credit Seats:    -(2500 × 16/31).round() = -1290
///   Charge Platform:  (2900 × 16/31).round() = 1497
///   Charge Seats:     (1000 × 16/31).round() = 516
///   Net: -5110 + -1290 + 1497 + 516 = -4387
#[rstest]
#[tokio::test]
async fn test_immediate_plan_change_downgrade(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    // Start on the more expensive Pro plan
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PRO_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    let result = env
        .services()
        .apply_plan_change_immediate_at(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_STARTER_ID,
            vec![],
            change_date,
        )
        .await
        .expect("immediate downgrade should succeed");

    assert!(
        result.adjustment_invoice_id.is_some(),
        "should create adjustment invoice for downgrade"
    );

    // Subscription should now be on Starter
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_STARTER_ID);

    // Adjustment should be negative (credit)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);

    // Negative total, zero amount_due, marked as Paid
    invoices
        .assert()
        .invoice_at(1)
        .with_context("downgrade adjustment")
        .has_total(-4387)
        .has_amount_due(0)
        .has_payment_status(meteroid_store::domain::InvoicePaymentStatus::Paid);

    // Customer balance should be credited with the absolute value of the negative total
    let sub = env.get_subscription(sub_id).await;
    let customer = env.get_customer(sub.customer_id).await;
    assert_eq!(
        customer.balance_value_cents, 4387,
        "customer balance should be credited 4387 cents from downgrade"
    );
}

/// Immediate change cancels any pending scheduled plan change.
#[rstest]
#[tokio::test]
async fn test_immediate_plan_change_cancels_pending_scheduled(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Schedule an end-of-period change first
    env.services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![])
        .await
        .expect("schedule should succeed");

    // Now apply immediate change (same target)
    env.services()
        .apply_plan_change_immediate_at(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![], change_date)
        .await
        .expect("immediate should succeed even with pending scheduled");

    // Pending scheduled events should be cancelled
    let mut conn = env.conn().await;
    let pending =
        diesel_models::scheduled_events::ScheduledEventRow::get_pending_events_for_subscription(
            &mut conn, sub_id, &TENANT_ID,
        )
        .await
        .expect("query pending events");

    assert!(
        pending.is_empty(),
        "pending scheduled events should be cancelled after immediate change"
    );
}

/// Preview with Immediate mode returns proration summary and change direction.
#[rstest]
#[tokio::test]
async fn test_preview_immediate_plan_change(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // preview_plan_change with Immediate mode uses Utc::now() as effective_date internally
    // (no _at variant available), so start_date must be today to keep the period current.
    let today = chrono::Utc::now().naive_utc().date();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(today)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let result = env
        .services()
        .preview_plan_change(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
            Some(meteroid_store::domain::subscription_changes::PlanChangeMode::Immediate),
        )
        .await
        .expect("preview failed");

    // Should detect upgrade direction
    assert_eq!(
        result.change_direction,
        meteroid_store::domain::subscription_changes::ChangeDirection::Upgrade,
        "Starter→Pro should be detected as Upgrade"
    );

    // Should include proration summary for Immediate mode
    let proration = result
        .proration
        .expect("proration should be present for Immediate mode");
    assert!(
        proration.charges_total_cents > 0,
        "charges should be positive"
    );
    assert!(
        proration.credits_total_cents < 0,
        "credits should be negative"
    );
    assert!(
        proration.net_amount_cents > 0,
        "net should be positive for upgrade"
    );
    assert!(proration.days_remaining > 0, "days_remaining should be > 0");
    assert!(proration.days_in_period > 0, "days_in_period should be > 0");
}

/// End-of-period plan change: verifies temporal component rotation.
/// Checks that old components are closed and new ones are created with correct dates.
#[rstest]
#[tokio::test]
async fn test_end_of_period_change_with_temporal_rotation(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let apply_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Verify initial Starter components
    let components = env.get_subscription_components(sub_id).await;
    assert_eq!(components.len(), 2, "Starter should have 2 components");
    let starter_comp_ids: Vec<_> = components.iter().map(|c| c.id).collect();

    // Schedule end-of-period change
    env.services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![])
        .await
        .expect("schedule_plan_change failed");

    // Process cycles to apply the change
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_PRO_ID);
    sub.assert().is_active().has_cycle_index(1);

    // Query ALL components (active + closed) to verify temporal rotation
    let all_components = env
        .get_all_subscription_components(sub_id, start_date, apply_date + chrono::Duration::days(1))
        .await;

    // Should have 4 total: 2 closed Starter + 2 active Pro
    assert_eq!(
        all_components.len(),
        4,
        "should have 4 total components (2 closed Starter + 2 active Pro)"
    );

    // Verify old Starter components are closed with effective_to = apply_date
    let closed: Vec<_> = all_components
        .iter()
        .filter(|c| c.effective_to.is_some())
        .collect();
    assert_eq!(closed.len(), 2, "should have 2 closed Starter components");
    for c in &closed {
        assert_eq!(
            c.effective_from, start_date,
            "closed component '{}' should have effective_from = start_date",
            c.name
        );
        assert_eq!(
            c.effective_to,
            Some(apply_date),
            "closed component '{}' should have effective_to = apply_date",
            c.name
        );
        assert!(
            starter_comp_ids.contains(&c.id),
            "closed component should be one of the original Starter components"
        );
    }

    // Verify new Pro components are active with effective_from = apply_date
    let active: Vec<_> = all_components
        .iter()
        .filter(|c| c.effective_to.is_none())
        .collect();
    assert_eq!(active.len(), 2, "should have 2 active Pro components");
    for c in &active {
        assert_eq!(
            c.effective_from, apply_date,
            "active component '{}' should have effective_from = apply_date",
            c.name
        );
        assert!(
            c.product_id.is_some(),
            "active component '{}' should have product_id",
            c.name
        );
    }

    // Renewal should bill at Pro prices
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("Pro renewal after end-of-period change")
        .is_finalized_unpaid()
        .has_total(12400);
}

// =============================================================================
// SLOT PRESERVATION
// =============================================================================

/// Slot count is preserved across end-of-period plan changes.
/// Also verifies MRR is updated correctly after the change.
///
/// Starter: Platform Fee €29 + Seats €10×10 = €129/mo (12900 cents)
/// Pro:     Platform Fee €99 + Seats €25×10 = €349/mo (34900 cents)
#[rstest]
#[tokio::test]
async fn test_plan_change_preserves_slot_count(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // --- Create Starter subscription (default 1 seat) ---
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(0);

    // Verify initial slot count = 1 (min_slots default)
    let slots = env
        .store()
        .get_active_slots_value(TENANT_ID, sub_id, "Seats".to_string(), None)
        .await
        .expect("get initial slots");
    assert_eq!(slots, 1, "should start with 1 seat (min_slots default)");

    // --- Add 9 more seats (total 10) ---
    env.services()
        .update_subscription_slots_for_test(
            TENANT_ID,
            sub_id,
            COMP_STARTER_SEATS_ID,
            9,
            SlotUpgradeBillingMode::Optimistic,
            NaiveDate::from_ymd_opt(2024, 1, 2).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("add 9 seats");

    let slots = env
        .store()
        .get_active_slots_value(TENANT_ID, sub_id, "Seats".to_string(), None)
        .await
        .expect("get slots after upgrade");
    assert_eq!(slots, 10, "should have 10 seats after upgrade");

    // --- Schedule plan change to Pro ---
    env.services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![])
        .await
        .expect("schedule_plan_change failed");

    // --- Process cycles: applies plan change and renews at Pro pricing ---
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(
        sub.plan_version_id, PLAN_VERSION_PRO_ID,
        "should be on Pro plan"
    );
    sub.assert().is_active().has_cycle_index(1);

    // --- Verify slot count is still 10 after plan change ---
    let slots = env
        .store()
        .get_active_slots_value(
            TENANT_ID,
            sub_id,
            "Seats".to_string(),
            NaiveDate::from_ymd_opt(2024, 2, 1).map(|t| t.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("get slots after plan change");
    assert_eq!(
        slots, 10,
        "slot count should be preserved across plan change"
    );

    // --- Verify Pro invoice bills for 10 seats ---
    // Pro: Platform Fee €99 (9900) + Seats €25×10 (25000) = €349 (34900)
    let invoices = env.get_invoices(sub_id).await;
    invoices
        .assert()
        .invoice_at(invoices.len() - 1)
        .with_context("Pro invoice should bill for 10 seats")
        .is_finalized_unpaid()
        .has_total(34900);

    // --- Verify MRR reflects 10 seats at Pro pricing ---
    // MRR = Platform Fee €99 (9900) + Seats €25×10 (25000) = €349 (34900)
    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .with_context("MRR after plan change with 10 seats")
        .has_mrr(34900);
}

/// Slot count is preserved across immediate plan changes,
/// and the adjustment invoice prorates based on the actual slot count.
///
/// Fixed dates: start Jan 1, period [Jan 1, Feb 1] = 31 days, change on Jan 16 → 16 days remaining.
/// factor = 16/31
///
/// Starter with 5 seats: Platform Fee €29 (2900) + Seats €10×5 (5000) = €79/mo (7900 cents)
/// Pro with 5 seats:     Platform Fee €99 (9900) + Seats €25×5 (12500) = €224/mo (22400 cents)
///
/// Expected adjustment (per-component rounding):
///   Credit Platform: -(2900 × 16/31).round() = -1497
///   Credit Seats:    -(5000 × 16/31).round() = -2581
///   Charge Platform:  (9900 × 16/31).round() = 5110
///   Charge Seats:    (12500 × 16/31).round() = 6452
///   Net: -1497 + -2581 + 5110 + 6452 = 7484
#[rstest]
#[tokio::test]
async fn test_immediate_plan_change_preserves_slot_count(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    // --- Create Starter subscription and add seats ---
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Add 4 more seats (1 default + 4 = 5 total)
    env.services()
        .update_subscription_slots_for_test(
            TENANT_ID,
            sub_id,
            COMP_STARTER_SEATS_ID,
            4,
            SlotUpgradeBillingMode::Optimistic,
            Some(start_date.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("add 4 seats");

    let slots = env
        .store()
        .get_active_slots_value(TENANT_ID, sub_id, "Seats".to_string(), None)
        .await
        .expect("get slots");
    assert_eq!(slots, 5, "should have 5 seats");

    // --- Immediate upgrade to Pro at Jan 16 ---
    let result = env
        .services()
        .apply_plan_change_immediate_at(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![], change_date)
        .await
        .expect("immediate plan change failed");

    assert!(
        result.adjustment_invoice_id.is_some(),
        "should create adjustment invoice"
    );

    // --- Verify slot count still 5 after plan change ---
    let slots = env
        .store()
        .get_active_slots_value(TENANT_ID, sub_id, "Seats".to_string(), None)
        .await
        .expect("get slots after immediate change");
    assert_eq!(
        slots, 5,
        "slot count should be preserved across immediate plan change"
    );

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_PRO_ID);

    // --- Verify MRR reflects 5 seats at Pro pricing ---
    // MRR = Platform Fee €99 (9900) + Seats €25×5 (12500) = €224 (22400)
    sub.assert()
        .with_context("MRR after immediate plan change with 5 seats")
        .has_mrr(22400);

    // --- Verify adjustment invoice prorates based on 5 seats ---
    // Hardcoded: 16 days remaining out of 31-day period (Jan 1→Feb 1), factor = 16/31
    // Credit Platform: -(2900 × 16/31).round() = -1497
    // Credit Seats:    -(5000 × 16/31).round() = -2581
    // Charge Platform:  (9900 × 16/31).round() = 5110
    // Charge Seats:    (12500 × 16/31).round() = 6452
    // Net: 7484
    let invoices = env.get_invoices(sub_id).await;
    let adj = &invoices[invoices.len() - 1];
    assert_eq!(
        adj.total, 7484,
        "adjustment should be 7484 cents for 5-seat Starter→Pro mid-period upgrade"
    );
}

// =============================================================================
// PLAN CHANGE: RATE-ONLY TO PLAN WITH SLOTS
// =============================================================================

/// Plan change from a rate-only plan (LeetCode) to a plan with slots (Starter),
/// providing initial_slot_count via ComponentParameterization.
///
/// LeetCode: €35/mo (rate only, no slots)
/// Starter: €29/mo platform fee + €10/seat (slots, min_slots=1)
///
/// Verifies:
/// - Adjustment invoice prorates correctly with added slot component
/// - Slot transactions are seeded with the provided initial_slot_count
/// - Next recurring invoice bills at the correct slot count
#[rstest]
#[tokio::test]
async fn test_plan_change_rate_only_to_plan_with_slots(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // 1. Subscribe to LeetCode (rate-only plan, €35/mo)
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // 2. Verify initial invoice (€35.00 = 3500 cents)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("LeetCode initial invoice")
        .is_finalized_unpaid()
        .has_total(3500);

    // 3. Schedule plan change to Starter with initial_slot_count = 5
    //    LeetCode products don't match Starter products, so:
    //    - Removed: LeetCode Rate (PRODUCT_LEETCODE_RATE_ID)
    //    - Added: Starter Platform Fee (PRODUCT_PLATFORM_FEE_ID)
    //    - Added: Starter Seats (PRODUCT_SEATS_ID) with 5 initial slots
    env.services()
        .schedule_plan_change(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_STARTER_ID,
            vec![ComponentParameterization {
                component_id: COMP_STARTER_SEATS_ID,
                parameters: ComponentParameters {
                    initial_slot_count: Some(5),
                    billing_period: None,
                    committed_capacity: None,
                },
            }],
        )
        .await
        .expect("schedule_plan_change failed");

    // 4. Process cycle → applies plan change at period boundary, then renews at Starter pricing
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(
        sub.plan_version_id, PLAN_VERSION_STARTER_ID,
        "plan should have changed to Starter"
    );
    sub.assert()
        .is_active()
        .has_cycle_index(1)
        .has_period_start(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
        .has_period_end(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap());

    // 5. Verify slot count = 5 (seeded from initial_slot_count on the Added component)
    let slots = env
        .store()
        .get_active_slots_value(
            TENANT_ID,
            sub_id,
            "Seats".to_string(),
            NaiveDate::from_ymd_opt(2024, 2, 1).map(|d| d.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("get slots after plan change");
    assert_eq!(slots, 5, "slot count should be 5 from initial_slot_count");

    // 6. Verify first Starter invoice bills at 5 seats
    //    Platform Fee: 2900 + Seats: 5 × 1000 = 7900
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("first Starter invoice with 5 seats")
        .is_finalized_unpaid()
        .has_total(7900)
        .has_period(
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        );
}

/// Immediate plan change from rate-only plan (LeetCode) to plan with slots (Starter),
/// providing initial_slot_count = 5 via ComponentParameterization.
///
/// Fixed dates: start Jan 1, period [Jan 1, Feb 1] = 31 days, change on Jan 16 → 16 days remaining.
/// factor = 16/31
///
/// LeetCode: €35/mo (rate only) → Starter: €29/mo platform fee + €10/seat
/// All components are added/removed (no product match), so:
///   Credit: -(3500 × 16/31).round() = -1806
///   Charge Platform: +(2900 × 16/31).round() = 1497
///   Charge Seats 5×€10: +(5000 × 16/31).round() = 2581
///   Net: -1806 + 1497 + 2581 = 2272
#[rstest]
#[tokio::test]
async fn test_immediate_plan_change_rate_only_to_plan_with_slots(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    // 1. Subscribe to LeetCode (rate-only plan, €35/mo)
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // 2. Verify initial invoice
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("LeetCode initial invoice")
        .is_finalized_unpaid()
        .has_total(3500);

    // 3. Immediate plan change to Starter with initial_slot_count = 5 at Jan 16
    let result = env
        .services()
        .apply_plan_change_immediate_at(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_STARTER_ID,
            vec![ComponentParameterization {
                component_id: COMP_STARTER_SEATS_ID,
                parameters: ComponentParameters {
                    initial_slot_count: Some(5),
                    billing_period: None,
                    committed_capacity: None,
                },
            }],
            change_date,
        )
        .await
        .expect("immediate plan change failed");

    assert!(
        result.adjustment_invoice_id.is_some(),
        "should create adjustment invoice"
    );

    // 5. Verify slot count = 5
    let slots = env
        .store()
        .get_active_slots_value(TENANT_ID, sub_id, "Seats".to_string(), None)
        .await
        .expect("get slots after plan change");
    assert_eq!(slots, 5, "slot count should be 5 from initial_slot_count");

    // 6. Verify adjustment invoice proration
    // Hardcoded: 16 days remaining out of 31-day period (Jan 1→Feb 1), factor = 16/31
    //   Credit LeetCode Rate: -(3500 × 16/31).round() = -1806
    //   Charge Starter Platform: (2900 × 16/31).round() = 1497
    //   Charge Starter Seats 5×€10: (5000 × 16/31).round() = 2581
    //   Net: 2272
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    assert_eq!(
        invoices[1].total, 2272,
        "adjustment should be 2272 cents for LeetCode→5-seat Starter mid-period change"
    );

    // 7. Verify subscription is now on Starter
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_STARTER_ID);
}

// =============================================================================
// USAGE TEMPORAL SPLIT
// =============================================================================

/// Helper to build a MockUsageClient with entries keyed on (metric_id, period_start, period_end).
fn build_usage_mock(entries: Vec<(MockUsageDataParams, Decimal)>) -> Arc<MockUsageClient> {
    let mut data = HashMap::new();
    for (params, value) in entries {
        let period = Period {
            start: params.period_start,
            end: params.period_end,
        };
        data.insert(
            params,
            UsageData {
                data: vec![GroupedUsageData {
                    value,
                    dimensions: HashMap::new(),
                }],
                period,
            },
        );
    }
    Arc::new(MockUsageClient { data })
}

/// Mid-period plan change with Usage components verifies temporal split billing.
///
/// Timeline (all fixed past dates):
/// Jan 1 2025:  Create sub on Usage Alpha. Period [Jan 1, Feb 1].
///              Invoice 0: Rate €10 advance only (no usage on cycle 0).
/// Feb 1:       process_cycles → Period [Feb 1, Mar 1].
///              Invoice 1: Rate €10 advance + Usage arrear [Jan 1, Feb 1].
/// Feb 15:      apply_plan_change_immediate_at(change_date = Feb 15)
///              Adjustment invoice: prorated Rate only, NO usage.
///              Old components closed at Feb 15, new from Feb 15.
/// Mar 1:       process_cycles → Period [Mar 1, Apr 1].
///              Invoice 2: Rate €20 advance + temporal split:
///                - Old "API Calls": [Feb 1, Feb 15] at €0.10/unit
///                - New "API Calls": [Feb 15, Mar 1] at €0.20/unit
///                - New "DB Storage": [Feb 15, Mar 1] at €0.50/unit
///
/// Usage Alpha: Rate €10/mo + Usage "API Calls" on METRIC_BANDWIDTH at €0.10/unit
/// Usage Beta:  Rate €20/mo + Usage "API Calls" on METRIC_BANDWIDTH at €0.20/unit
///                          + Usage "DB Storage" on METRIC_DATABASE_SIZE at €0.50/unit
#[tokio::test]
async fn test_immediate_plan_change_usage_temporal_split() {
    let jan1 = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
    let feb15 = NaiveDate::from_ymd_opt(2025, 2, 15).unwrap();
    let mar1 = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();

    let usage_client = build_usage_mock(vec![
        // Cycle 1 arrear: full period Jan 1 → Feb 1
        (
            MockUsageDataParams {
                metric_id: METRIC_BANDWIDTH,
                period_start: jan1,
                period_end: feb1,
            },
            Decimal::new(1000, 0), // 1000 units
        ),
        // Old API Calls: temporal split Feb 1 → Feb 15
        (
            MockUsageDataParams {
                metric_id: METRIC_BANDWIDTH,
                period_start: feb1,
                period_end: feb15,
            },
            Decimal::new(50, 0), // 50 units
        ),
        // New API Calls: temporal split Feb 15 → Mar 1
        (
            MockUsageDataParams {
                metric_id: METRIC_BANDWIDTH,
                period_start: feb15,
                period_end: mar1,
            },
            Decimal::new(200, 0), // 200 units
        ),
        // New DB Storage: only active after change, Feb 15 → Mar 1
        (
            MockUsageDataParams {
                metric_id: METRIC_DATABASE_SIZE,
                period_start: feb15,
                period_end: mar1,
            },
            Decimal::new(100, 0), // 100 units
        ),
    ]);

    let env = test_env_with_usage(usage_client).await;

    // --- Cycle 0: Subscribe on Usage Alpha at Jan 1 ---
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ALPHA_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(0)
        .has_period_start(jan1)
        .has_period_end(feb1);

    // Invoice 0: Rate €10 advance only (no usage on cycle 0)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("cycle 0 - Alpha initial invoice")
        .is_finalized_unpaid()
        .has_total(1000); // €10 = 1000 cents

    // --- Cycle 1: process_cycles → Period [Feb 1, Mar 1] ---
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(1)
        .has_period_start(feb1)
        .has_period_end(mar1);

    // Invoice 1: Rate €10 advance + Usage arrear 1000 × €0.10 = €100
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("cycle 1 - Alpha invoice (rate + usage arrear)")
        .is_finalized_unpaid()
        .has_total(1000 + 10000); // €10 rate + €100 usage = €110 = 11000 cents

    // --- Feb 15: Immediate plan change Usage Alpha → Usage Beta ---
    let result = env
        .services()
        .apply_plan_change_immediate_at(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_USAGE_BETA_ID,
            vec![],
            feb15,
        )
        .await
        .expect("apply_plan_change_immediate_at failed");

    assert!(
        result.adjustment_invoice_id.is_some(),
        "should create adjustment invoice for rate proration"
    );

    // Verify adjustment invoice has NO usage line items (usage excluded from proration)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    let adj = &invoices[2];
    for line in &adj.line_items {
        assert!(
            line.metric_id.is_none(),
            "adjustment invoice should not have usage lines, found: {}",
            line.name
        );
    }

    // Verify adjustment: prorated Rate only
    // 14 days remaining out of 28 days (Feb 1→Mar 1), factor = 14/28 = 0.5
    //   Credit old rate €10: -(1000 × 0.5) = -500
    //   Charge new rate €20: +(2000 × 0.5) = 1000
    //   Net: 500
    assert_eq!(
        adj.total, 500,
        "adjustment should be 500 cents for prorated Rate upgrade (€10→€20, half period)"
    );

    // --- Verify component temporal bounds after Feb 15 change ---
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_USAGE_BETA_ID);

    let all_components = env
        .get_all_subscription_components(sub_id, jan1, mar1)
        .await;

    // Old Alpha: Platform Fee + API Calls (closed)
    // New Beta: Platform Fee + API Calls + DB Storage (active)
    assert_eq!(
        all_components.len(),
        5,
        "should have 5 total components (2 closed Alpha + 3 active Beta)"
    );

    let closed: Vec<_> = all_components
        .iter()
        .filter(|c| c.effective_to.is_some())
        .collect();
    assert_eq!(closed.len(), 2, "should have 2 closed Alpha components");
    for c in &closed {
        assert_eq!(
            c.effective_from, jan1,
            "closed component '{}' should have effective_from = Jan 1",
            c.name
        );
        assert_eq!(
            c.effective_to,
            Some(feb15),
            "closed component '{}' should have effective_to = Feb 15",
            c.name
        );
    }

    let active: Vec<_> = all_components
        .iter()
        .filter(|c| c.effective_to.is_none())
        .collect();
    assert_eq!(active.len(), 3, "should have 3 active Beta components");
    for c in &active {
        assert_eq!(
            c.effective_from, feb15,
            "active component '{}' should have effective_from = Feb 15",
            c.name
        );
    }

    // --- Cycle 2: process_cycles → Period [Mar 1, Apr 1] ---
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(2)
        .has_period_start(mar1);

    // Invoice 3: Rate €20 advance + temporal split usage
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(4);

    let inv = &invoices[3];

    // Expected line items:
    // 1. Rate €20/mo advance = 2000 cents
    // 2. Old "API Calls" arrear [Feb 1, Feb 15]: 50 × €0.10 = 500 cents
    // 3. New "API Calls" arrear [Feb 15, Mar 1]: 200 × €0.20 = 4000 cents
    // 4. "DB Storage" arrear [Feb 15, Mar 1]: 100 × €0.50 = 5000 cents
    // Total: 2000 + 500 + 4000 + 5000 = 11500 cents
    assert_eq!(
        inv.total, 11500,
        "invoice 3 total should be 11500 (rate 2000 + old API 500 + new API 4000 + DB 5000)"
    );

    // Verify individual line items
    let rate_lines: Vec<_> = inv
        .line_items
        .iter()
        .filter(|l| l.metric_id.is_none())
        .collect();
    assert_eq!(rate_lines.len(), 1, "should have 1 rate line item");
    assert_eq!(
        rate_lines[0].amount_subtotal, 2000,
        "rate advance should be €20"
    );

    let usage_lines: Vec<_> = inv
        .line_items
        .iter()
        .filter(|l| l.metric_id.is_some())
        .collect();
    assert_eq!(
        usage_lines.len(),
        3,
        "should have 3 usage line items (temporal split)"
    );

    // Check bandwidth (API Calls) lines have temporal split naming
    let bandwidth_lines: Vec<_> = usage_lines
        .iter()
        .filter(|l| l.metric_id == Some(METRIC_BANDWIDTH))
        .collect();
    assert_eq!(
        bandwidth_lines.len(),
        2,
        "should have 2 bandwidth lines (old + new API Calls)"
    );

    let db_lines: Vec<_> = usage_lines
        .iter()
        .filter(|l| l.metric_id == Some(METRIC_DATABASE_SIZE))
        .collect();
    assert_eq!(db_lines.len(), 1, "should have 1 DB Storage line");
    assert_eq!(
        db_lines[0].amount_subtotal, 5000,
        "DB Storage: 100 × €0.50 = 5000"
    );

    // Check the two bandwidth usage amounts sum to expected
    let bandwidth_total: i64 = bandwidth_lines.iter().map(|l| l.amount_subtotal).sum();
    assert_eq!(
        bandwidth_total, 4500,
        "bandwidth total should be 500 + 4000 = 4500"
    );
}

/// Complementary test: compute_upcoming_invoice after a mid-period usage plan change.
///
/// Same setup as the temporal split test but uses `compute_upcoming_invoice` to preview
/// the Mar 1 invoice without going through the full cycle pipeline.
#[tokio::test]
async fn test_immediate_plan_change_usage_upcoming_invoice() {
    let jan1 = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
    let feb15 = NaiveDate::from_ymd_opt(2025, 2, 15).unwrap();
    let mar1 = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();

    let usage_client = build_usage_mock(vec![
        // Cycle 1 arrear (not relevant for upcoming, but needed for cycle 1 invoice)
        (
            MockUsageDataParams {
                metric_id: METRIC_BANDWIDTH,
                period_start: jan1,
                period_end: feb1,
            },
            Decimal::new(1000, 0),
        ),
        // Old API Calls: temporal split Feb 1 → Feb 15
        (
            MockUsageDataParams {
                metric_id: METRIC_BANDWIDTH,
                period_start: feb1,
                period_end: feb15,
            },
            Decimal::new(50, 0),
        ),
        // New API Calls: temporal split Feb 15 → Mar 1
        (
            MockUsageDataParams {
                metric_id: METRIC_BANDWIDTH,
                period_start: feb15,
                period_end: mar1,
            },
            Decimal::new(200, 0),
        ),
        // New DB Storage: Feb 15 → Mar 1
        (
            MockUsageDataParams {
                metric_id: METRIC_DATABASE_SIZE,
                period_start: feb15,
                period_end: mar1,
            },
            Decimal::new(100, 0),
        ),
    ]);

    let env = test_env_with_usage(usage_client).await;

    // --- Cycle 0: Subscribe on Usage Alpha at Jan 1 ---
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ALPHA_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // --- Cycle 1: process_cycles → Period [Feb 1, Mar 1] ---
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .has_cycle_index(1)
        .has_period_start(feb1)
        .has_period_end(mar1);

    // --- Feb 15: Immediate plan change Usage Alpha → Usage Beta ---
    env.services()
        .apply_plan_change_immediate_at(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_USAGE_BETA_ID,
            vec![],
            feb15,
        )
        .await
        .expect("apply_plan_change_immediate_at failed");

    // --- Compute upcoming invoice (preview for Mar 1) ---
    let sub_details = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("get_subscription_details failed");

    let upcoming = env
        .services()
        .compute_upcoming_invoice(&sub_details)
        .await
        .expect("compute_upcoming_invoice failed");

    // Expected: same totals as the pipeline test
    // Rate €20/mo advance = 2000
    // Old API Calls [Feb 1, Feb 15]: 50 × €0.10 = 500
    // New API Calls [Feb 15, Mar 1]: 200 × €0.20 = 4000
    // DB Storage [Feb 15, Mar 1]: 100 × €0.50 = 5000
    // Total: 11500
    let total: i64 = upcoming
        .invoice_lines
        .iter()
        .map(|l| l.amount_subtotal)
        .sum();
    assert_eq!(
        total, 11500,
        "upcoming invoice total should be 11500 (rate 2000 + old API 500 + new API 4000 + DB 5000)"
    );

    // Verify line item count: 1 rate + 3 usage (temporal split)
    let rate_lines: Vec<_> = upcoming
        .invoice_lines
        .iter()
        .filter(|l| l.metric_id.is_none())
        .collect();
    assert_eq!(rate_lines.len(), 1, "should have 1 rate line item");

    let usage_lines: Vec<_> = upcoming
        .invoice_lines
        .iter()
        .filter(|l| l.metric_id.is_some())
        .collect();
    assert_eq!(
        usage_lines.len(),
        3,
        "should have 3 usage line items (temporal split)"
    );

    let bandwidth_lines: Vec<_> = usage_lines
        .iter()
        .filter(|l| l.metric_id == Some(METRIC_BANDWIDTH))
        .collect();
    assert_eq!(bandwidth_lines.len(), 2, "should have 2 bandwidth lines");

    let db_lines: Vec<_> = usage_lines
        .iter()
        .filter(|l| l.metric_id == Some(METRIC_DATABASE_SIZE))
        .collect();
    assert_eq!(db_lines.len(), 1, "should have 1 DB Storage line");
}

// =============================================================================
// MIXED FEE TYPE PLAN CHANGE
// =============================================================================
//
// Tests for plan changes involving Capacity, ExtraRecurring/Arrears, OneTime,
// and ExtraRecurring/Advance fee types. These exercise `is_pure_arrears()` for
// all non-Usage fee types.

// ── Test-local plan IDs (not in ids.rs since plans are created inline) ───────

// Mixed Alpha: Capacity + Rec/Arrears + OneTime + ExtraRec/Advance
const PLAN_MIXED_ALPHA_ID: PlanId =
    PlanId::from_const(uuid!("019438e0-0100-7000-8000-000000000001"));
const PLAN_VERSION_MIXED_ALPHA_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("019438e0-0101-7000-8000-000000000001"));
const COMP_ALPHA_CAPACITY_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0102-7000-8000-000000000001"));
const COMP_ALPHA_REC_ARREARS_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0103-7000-8000-000000000001"));
const COMP_ALPHA_ONETIME_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0104-7000-8000-000000000001"));
const COMP_ALPHA_EXTRA_ADVANCE_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0105-7000-8000-000000000001"));
const PRICE_ALPHA_CAPACITY_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0106-7000-8000-000000000001"));
const PRICE_ALPHA_REC_ARREARS_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0107-7000-8000-000000000001"));
const PRICE_ALPHA_ONETIME_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0108-7000-8000-000000000001"));
const PRICE_ALPHA_EXTRA_ADVANCE_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0109-7000-8000-000000000001"));

// Mixed Beta: Capacity + Rec/Arrears + ExtraRec/Advance (no OneTime)
const PLAN_MIXED_BETA_ID: PlanId =
    PlanId::from_const(uuid!("019438e0-0110-7000-8000-000000000001"));
const PLAN_VERSION_MIXED_BETA_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("019438e0-0111-7000-8000-000000000001"));
const COMP_BETA_CAPACITY_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0112-7000-8000-000000000001"));
const COMP_BETA_REC_ARREARS_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0113-7000-8000-000000000001"));
const COMP_BETA_EXTRA_ADVANCE_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0114-7000-8000-000000000001"));
const PRICE_BETA_CAPACITY_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0115-7000-8000-000000000001"));
const PRICE_BETA_REC_ARREARS_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0116-7000-8000-000000000001"));
const PRICE_BETA_EXTRA_ADVANCE_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0117-7000-8000-000000000001"));

// Mixed Capacity Upgraded: same structure as Alpha, different Capacity config
const PLAN_MIXED_CAP_UPG_ID: PlanId =
    PlanId::from_const(uuid!("019438e0-0120-7000-8000-000000000001"));
const PLAN_VERSION_MIXED_CAP_UPG_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("019438e0-0121-7000-8000-000000000001"));
const COMP_CAP_UPG_CAPACITY_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0122-7000-8000-000000000001"));
const COMP_CAP_UPG_REC_ARREARS_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0123-7000-8000-000000000001"));
const COMP_CAP_UPG_ONETIME_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0124-7000-8000-000000000001"));
const COMP_CAP_UPG_EXTRA_ADVANCE_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0125-7000-8000-000000000001"));
const PRICE_CAP_UPG_CAPACITY_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0126-7000-8000-000000000001"));
const PRICE_CAP_UPG_REC_ARREARS_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0127-7000-8000-000000000001"));
const PRICE_CAP_UPG_ONETIME_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0128-7000-8000-000000000001"));
const PRICE_CAP_UPG_EXTRA_ADVANCE_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0129-7000-8000-000000000001"));

// ── Inline plan builders ─────────────────────────────────────────────────────

fn mixed_alpha_plan() -> PlanSeed {
    PlanSeed::new(
        PLAN_MIXED_ALPHA_ID,
        "Mixed Alpha",
        PLAN_VERSION_MIXED_ALPHA_ID,
    )
    .components(vec![
        SeedComp::capacity(
            COMP_ALPHA_CAPACITY_ID,
            "Capacity Bandwidth",
            PRODUCT_CAPACITY_ID,
            METRIC_BANDWIDTH,
            PRICE_ALPHA_CAPACITY_ID,
            DieselBillingPeriodEnum::Monthly,
            Decimal::new(5000, 2), // €50 base
            100,                   // 100 included
            Decimal::new(5, 2),    // €0.05 overage
        ),
        SeedComp::extra_recurring(
            COMP_ALPHA_REC_ARREARS_ID,
            "Recurring Arrears",
            PRODUCT_RECURRING_ARREARS_ID,
            PRICE_ALPHA_REC_ARREARS_ID,
            DieselBillingPeriodEnum::Monthly,
            Decimal::new(1500, 2), // €15
            2,
            BillingType::Arrears,
        ),
        SeedComp::one_time(
            COMP_ALPHA_ONETIME_ID,
            "Setup Fee",
            PRODUCT_ONETIME_SETUP_ID,
            PRICE_ALPHA_ONETIME_ID,
            Decimal::new(20000, 2), // €200
            1,
        ),
        SeedComp::extra_recurring(
            COMP_ALPHA_EXTRA_ADVANCE_ID,
            "Extra Advance",
            PRODUCT_EXTRA_ADVANCE_ID,
            PRICE_ALPHA_EXTRA_ADVANCE_ID,
            DieselBillingPeriodEnum::Monthly,
            Decimal::new(3000, 2), // €30
            1,
            BillingType::Advance,
        ),
    ])
}

fn mixed_beta_plan() -> PlanSeed {
    PlanSeed::new(PLAN_MIXED_BETA_ID, "Mixed Beta", PLAN_VERSION_MIXED_BETA_ID).components(vec![
        SeedComp::capacity(
            COMP_BETA_CAPACITY_ID,
            "Capacity Bandwidth",
            PRODUCT_CAPACITY_ID,
            METRIC_BANDWIDTH,
            PRICE_BETA_CAPACITY_ID,
            DieselBillingPeriodEnum::Monthly,
            Decimal::new(8000, 2), // €80 base
            200,                   // 200 included
            Decimal::new(10, 2),   // €0.10 overage
        ),
        SeedComp::extra_recurring(
            COMP_BETA_REC_ARREARS_ID,
            "Recurring Arrears",
            PRODUCT_RECURRING_ARREARS_ID,
            PRICE_BETA_REC_ARREARS_ID,
            DieselBillingPeriodEnum::Monthly,
            Decimal::new(2500, 2), // €25
            2,
            BillingType::Arrears,
        ),
        SeedComp::extra_recurring(
            COMP_BETA_EXTRA_ADVANCE_ID,
            "Extra Advance",
            PRODUCT_EXTRA_ADVANCE_ID,
            PRICE_BETA_EXTRA_ADVANCE_ID,
            DieselBillingPeriodEnum::Monthly,
            Decimal::new(5000, 2), // €50
            1,
            BillingType::Advance,
        ),
    ])
}

fn mixed_cap_upgraded_plan() -> PlanSeed {
    PlanSeed::new(
        PLAN_MIXED_CAP_UPG_ID,
        "Mixed Cap Upgraded",
        PLAN_VERSION_MIXED_CAP_UPG_ID,
    )
    .components(vec![
        SeedComp::capacity(
            COMP_CAP_UPG_CAPACITY_ID,
            "Capacity Bandwidth",
            PRODUCT_CAPACITY_ID,
            METRIC_BANDWIDTH,
            PRICE_CAP_UPG_CAPACITY_ID,
            DieselBillingPeriodEnum::Monthly,
            Decimal::new(12000, 2), // €120 base
            500,                    // 500 included
            Decimal::new(2, 2),     // €0.02 overage
        ),
        SeedComp::extra_recurring(
            COMP_CAP_UPG_REC_ARREARS_ID,
            "Recurring Arrears",
            PRODUCT_RECURRING_ARREARS_ID,
            PRICE_CAP_UPG_REC_ARREARS_ID,
            DieselBillingPeriodEnum::Monthly,
            Decimal::new(1500, 2), // €15 (same as Alpha)
            2,
            BillingType::Arrears,
        ),
        SeedComp::one_time(
            COMP_CAP_UPG_ONETIME_ID,
            "Setup Fee",
            PRODUCT_ONETIME_SETUP_ID,
            PRICE_CAP_UPG_ONETIME_ID,
            Decimal::new(20000, 2), // €200 (same as Alpha)
            1,
        ),
        SeedComp::extra_recurring(
            COMP_CAP_UPG_EXTRA_ADVANCE_ID,
            "Extra Advance",
            PRODUCT_EXTRA_ADVANCE_ID,
            PRICE_CAP_UPG_EXTRA_ADVANCE_ID,
            DieselBillingPeriodEnum::Monthly,
            Decimal::new(3000, 2), // €30 (same as Alpha)
            1,
            BillingType::Advance,
        ),
    ])
}

// ── Test 1: Alpha → Beta (upgrade, mixed fee types) ─────────────────────────

/// Mid-period plan change from Mixed Alpha (4 components) to Mixed Beta (3 components).
///
/// Verifies:
/// - Adjustment invoice prorates only advance-billed components (Capacity base, ExtraRec/Advance)
/// - Rec/Arrears is excluded from adjustment (is_pure_arrears) and gets temporal split
/// - OneTime is excluded from adjustment (advance_amount = 0) and not re-charged
/// - Component temporal bounds are correct after plan change
#[tokio::test]
async fn test_immediate_plan_change_mixed_alpha_to_beta() {
    let jan1 = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
    let feb15 = NaiveDate::from_ymd_opt(2025, 2, 15).unwrap();
    let mar1 = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();

    let env = test_env_with_usage(build_usage_mock(vec![])).await;

    // Seed inline plans
    let mut conn = env.conn().await;
    mixed_alpha_plan().seed(&mut conn).await.unwrap();
    mixed_beta_plan().seed(&mut conn).await.unwrap();
    drop(conn);

    // --- Cycle 0: Subscribe on Mixed Alpha at Jan 1 ---
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_MIXED_ALPHA_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(0)
        .has_period_start(jan1)
        .has_period_end(feb1);

    // Invoice 0: Capacity base €50 + ExtraRec/Advance €30 + OneTime €200 = €280
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("cycle 0 - Alpha initial invoice")
        .is_finalized_unpaid()
        .has_total(5000 + 3000 + 20000);

    // --- Cycle 1: process_cycles → Period [Feb 1, Mar 1] ---
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(1)
        .has_period_start(feb1)
        .has_period_end(mar1);

    // Invoice 1: Capacity base €50 + ExtraRec/Advance €30 + Rec/Arrears €30 = €110
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("cycle 1 - Alpha (advance + arrears from cycle 0)")
        .is_finalized_unpaid()
        .has_total(5000 + 3000 + 3000);

    // --- Feb 15: Immediate plan change Alpha → Beta ---
    let result = env
        .services()
        .apply_plan_change_immediate_at(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_MIXED_BETA_ID,
            vec![],
            feb15,
        )
        .await
        .expect("apply_plan_change_immediate_at failed");

    assert!(
        result.adjustment_invoice_id.is_some(),
        "should create adjustment invoice"
    );

    // Adjustment invoice: prorate advance-only components
    // Period [Feb 1, Mar 1] = 28 days, remaining after Feb 15 = 14 days, factor = 0.5
    //   Credit Capacity:       -(5000 × 0.5) = -2500
    //   Charge Capacity:        (8000 × 0.5) = 4000
    //   Credit ExtraRec/Adv:   -(3000 × 0.5) = -1500
    //   Charge ExtraRec/Adv:    (5000 × 0.5) = 2500
    //   Rec/Arrears: excluded (is_pure_arrears)
    //   OneTime: excluded (advance_amount = 0)
    //   Net: -2500 + 4000 + -1500 + 2500 = 2500
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    assert_eq!(
        invoices[2].total, 2500,
        "adjustment should be 2500 cents for prorated Capacity base + ExtraRec/Advance upgrade"
    );

    // Verify subscription switched to Beta
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_MIXED_BETA_ID);

    // Verify component temporal bounds
    let all_components = env
        .get_all_subscription_components(sub_id, jan1, mar1)
        .await;

    // Old Alpha: 4 closed, New Beta: 3 active = 7 total
    assert_eq!(
        all_components.len(),
        7,
        "should have 7 total components (4 closed Alpha + 3 active Beta)"
    );

    let closed: Vec<_> = all_components
        .iter()
        .filter(|c| c.effective_to.is_some())
        .collect();
    assert_eq!(closed.len(), 4, "should have 4 closed Alpha components");
    for c in &closed {
        assert_eq!(
            c.effective_to,
            Some(feb15),
            "closed component '{}' effective_to",
            c.name
        );
    }

    let active: Vec<_> = all_components
        .iter()
        .filter(|c| c.effective_to.is_none())
        .collect();
    assert_eq!(active.len(), 3, "should have 3 active Beta components");
    for c in &active {
        assert_eq!(
            c.effective_from, feb15,
            "active component '{}' effective_from",
            c.name
        );
    }

    // --- Cycle 2: process_cycles → verify temporal split for Rec/Arrears ---
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(2)
        .has_period_start(mar1);

    // Invoice 3: Beta advance + Rec/Arrears temporal split
    // Capacity base €80 = 8000 + ExtraRec/Advance €50 = 5000
    // + Old Rec/Arrears [Feb 1, Feb 15] = 3000 + New Rec/Arrears [Feb 15, Mar 1] = 5000
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(4);
    assert_eq!(
        invoices[3].total,
        8000 + 5000 + 3000 + 5000,
        "cycle 2 invoice: Beta advance + temporal split Rec/Arrears"
    );
}

// ── Test 2: Beta → Alpha (end-of-period downgrade with OneTime) ──────────────

/// End-of-period plan change from Mixed Beta (3 components) to Mixed Alpha (4 components).
/// Immediate downgrades are rejected, so this uses `schedule_plan_change`.
///
/// Verifies:
/// - Scheduled downgrade applies at period boundary (no mid-period proration)
/// - OneTime NOT charged (subscription cycle_index > 0, so OneTime period is skipped)
/// - Rec/Arrears from old plan is billed as arrear on the transition invoice
/// - Component rotation: old components closed, new components start at boundary
#[tokio::test]
async fn test_scheduled_plan_change_mixed_beta_to_alpha() {
    let jan1 = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
    let mar1 = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();

    let env = test_env_with_usage(build_usage_mock(vec![])).await;

    // Seed inline plans
    let mut conn = env.conn().await;
    mixed_beta_plan().seed(&mut conn).await.unwrap();
    mixed_alpha_plan().seed(&mut conn).await.unwrap();
    drop(conn);

    // --- Cycle 0: Subscribe on Mixed Beta at Jan 1 ---
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_MIXED_BETA_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(0)
        .has_period_start(jan1)
        .has_period_end(feb1);

    // Invoice 0: Capacity base €80 + ExtraRec/Advance €50 = €130 (no OneTime on Beta)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("cycle 0 - Beta initial invoice")
        .is_finalized_unpaid()
        .has_total(8000 + 5000);

    // --- Schedule downgrade to Alpha (end-of-period) ---
    env.services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_MIXED_ALPHA_ID, vec![])
        .await
        .expect("schedule_plan_change failed");

    // --- Cycle 1: process_cycles applies scheduled change + renews ---
    // Plan change applied at Feb 1 boundary, new period starts on Alpha
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(1)
        .has_period_start(feb1)
        .has_period_end(mar1);
    assert_eq!(
        sub.plan_version_id, PLAN_VERSION_MIXED_ALPHA_ID,
        "subscription should be on Alpha after scheduled change"
    );

    // Invoice 1 (transition): Beta arrears + Alpha advance, NO OneTime
    // Beta Rec/Arrears €50 (arrear from cycle 0) = 5000
    // Alpha Capacity base €50 (advance) = 5000
    // Alpha ExtraRec/Advance €30 (advance) = 3000
    // OneTime NOT charged (subscription cycle_index=1, OneTime billing period already past)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("cycle 1 - transition (Beta arrear + Alpha advance, no OneTime)")
        .is_finalized_unpaid()
        .has_total(5000 + 5000 + 3000);

    // Verify OneTime is NOT charged (subscription is past cycle 0)
    let onetime_lines: Vec<_> = invoices[1]
        .line_items
        .iter()
        .filter(|l| l.name.contains("Setup Fee"))
        .collect();
    assert!(
        onetime_lines.is_empty(),
        "OneTime should NOT be charged (subscription cycle_index > 0)"
    );

    // Verify component rotation: Beta closed at Feb 1, Alpha active from Feb 1
    let all_components = env
        .get_all_subscription_components(sub_id, jan1, mar1)
        .await;
    assert_eq!(all_components.len(), 7, "3 closed Beta + 4 active Alpha");

    let closed: Vec<_> = all_components
        .iter()
        .filter(|c| c.effective_to.is_some())
        .collect();
    assert_eq!(closed.len(), 3, "3 closed Beta components");
    for c in &closed {
        assert_eq!(
            c.effective_to,
            Some(feb1),
            "closed component '{}' effective_to = Feb 1",
            c.name
        );
    }

    let active: Vec<_> = all_components
        .iter()
        .filter(|c| c.effective_to.is_none())
        .collect();
    assert_eq!(active.len(), 4, "4 active Alpha components");
    for c in &active {
        assert_eq!(
            c.effective_from, feb1,
            "active component '{}' effective_from = Feb 1",
            c.name
        );
    }

    // --- Cycle 2: verify no duplicate OneTime ---
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(2)
        .has_period_start(mar1);

    // Invoice 2: Alpha advance + Rec/Arrears arrear, NO OneTime
    // Capacity base €50 = 5000 + ExtraRec/Advance €30 = 3000 + Rec/Arrears €30 = 3000
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    invoices
        .assert()
        .invoice_at(2)
        .with_context("cycle 2 - Alpha steady state, no OneTime")
        .is_finalized_unpaid()
        .has_total(5000 + 3000 + 3000);

    // Confirm no OneTime on cycle 2
    let onetime_lines: Vec<_> = invoices[2]
        .line_items
        .iter()
        .filter(|l| l.name.contains("Setup Fee"))
        .collect();
    assert!(
        onetime_lines.is_empty(),
        "OneTime should NOT appear on cycle 2 (not first period)"
    );
}

// ── Test 2b: Beta → Alpha immediate downgrade (currently rejected) ────────────

/// Immediate downgrade from Mixed Beta to Mixed Alpha.
/// Lower total advance amount → negative adjustment invoice (credit > charge).
///
/// Verifies:
/// - Negative adjustment invoice (credit > charge)
/// - Rec/Arrears temporal split works in downgrade direction
/// - OneTime added but NOT charged (is_first_period = false, since arrear period exists)
#[tokio::test]
async fn test_immediate_plan_change_mixed_beta_to_alpha_downgrade() {
    let jan1 = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
    let feb15 = NaiveDate::from_ymd_opt(2025, 2, 15).unwrap();
    let mar1 = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();

    let env = test_env_with_usage(build_usage_mock(vec![])).await;

    // Seed inline plans
    let mut conn = env.conn().await;
    mixed_beta_plan().seed(&mut conn).await.unwrap();
    mixed_alpha_plan().seed(&mut conn).await.unwrap();
    drop(conn);

    // --- Cycle 0: Subscribe on Mixed Beta at Jan 1 ---
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_MIXED_BETA_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(0)
        .has_period_start(jan1)
        .has_period_end(feb1);

    // Invoice 0: Capacity base €80 + ExtraRec/Advance €50 = €130 (no OneTime on Beta)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("cycle 0 - Beta initial invoice")
        .is_finalized_unpaid()
        .has_total(8000 + 5000);

    // --- Cycle 1: process_cycles → Period [Feb 1, Mar 1] ---
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(1)
        .has_period_start(feb1)
        .has_period_end(mar1);

    // Invoice 1: Capacity base €80 + ExtraRec/Advance €50 + Rec/Arrears €50 = €180
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("cycle 1 - Beta (advance + arrears from cycle 0)")
        .is_finalized_unpaid()
        .has_total(8000 + 5000 + 5000);

    // --- Feb 15: Immediate plan change Beta → Alpha (downgrade) ---
    let result = env
        .services()
        .apply_plan_change_immediate_at(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_MIXED_ALPHA_ID,
            vec![],
            feb15,
        )
        .await
        .expect("apply_plan_change_immediate_at failed");

    assert!(
        result.adjustment_invoice_id.is_some(),
        "should create adjustment invoice for downgrade"
    );

    // Adjustment: negative (downgrade)
    // Period [Feb 1, Mar 1] = 28 days, remaining after Feb 15 = 14 days, factor = 0.5
    //   Credit Capacity:       -(8000 × 0.5) = -4000
    //   Charge Capacity:        (5000 × 0.5) = 2500
    //   Credit ExtraRec/Adv:   -(5000 × 0.5) = -2500
    //   Charge ExtraRec/Adv:    (3000 × 0.5) = 1500
    //   OneTime: added but advance_amount = 0 → no charge
    //   Net: -4000 + 2500 + -2500 + 1500 = -2500
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    assert_eq!(
        invoices[2].total, -2500,
        "adjustment should be -2500 cents for prorated downgrade"
    );

    // Verify subscription switched to Alpha
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_MIXED_ALPHA_ID);

    // Verify component counts: 3 closed Beta + 4 active Alpha = 7
    let all_components = env
        .get_all_subscription_components(sub_id, jan1, mar1)
        .await;
    assert_eq!(all_components.len(), 7, "3 closed Beta + 4 active Alpha");

    let active: Vec<_> = all_components
        .iter()
        .filter(|c| c.effective_to.is_none())
        .collect();
    assert_eq!(
        active.len(),
        4,
        "4 active Alpha components (including OneTime)"
    );

    // --- Cycle 2: process_cycles ---
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(2)
        .has_period_start(mar1);

    // Invoice 3: Alpha advance + temporal split Rec/Arrears + NO OneTime
    // Capacity base €50 = 5000 + ExtraRec/Advance €30 = 3000
    // + Old Rec/Arrears [Feb 1, Feb 15] = 5000 + New Rec/Arrears [Feb 15, Mar 1] = 3000
    // OneTime NOT charged (is_first_period = false)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(4);
    assert_eq!(
        invoices[3].total,
        5000 + 3000 + 5000 + 3000,
        "cycle 2: Alpha advance + temporal Rec/Arrears, OneTime NOT charged"
    );

    // Verify no OneTime line item on cycle 2 invoice
    let onetime_lines: Vec<_> = invoices[3]
        .line_items
        .iter()
        .filter(|l| l.name.contains("Setup Fee"))
        .collect();
    assert!(
        onetime_lines.is_empty(),
        "OneTime should NOT appear on cycle 2 invoice (is_first_period=false)"
    );
}

// ── Test 3: Alpha → Capacity Upgraded (same products, different config) ──────

/// Mid-period plan change from Mixed Alpha to Mixed Capacity Upgraded.
/// Same products in both plans, only Capacity config differs.
///
/// Verifies:
/// - Capacity base rate is prorated (different rates)
/// - Matched non-Capacity components with same prices produce zero net proration
/// - New Capacity overage uses updated included/overage_rate
#[tokio::test]
async fn test_immediate_plan_change_capacity_tier_upgrade() {
    let jan1 = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let feb1 = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
    let feb15 = NaiveDate::from_ymd_opt(2025, 2, 15).unwrap();
    let mar1 = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();

    // Mock usage: 600 bandwidth units in [Feb 15, Mar 1] for new Capacity overage
    let usage_client = build_usage_mock(vec![(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: feb15,
            period_end: mar1,
        },
        Decimal::new(600, 0),
    )]);

    let env = test_env_with_usage(usage_client).await;

    // Seed inline plans
    let mut conn = env.conn().await;
    mixed_alpha_plan().seed(&mut conn).await.unwrap();
    mixed_cap_upgraded_plan().seed(&mut conn).await.unwrap();
    drop(conn);

    // --- Cycle 0: Subscribe on Mixed Alpha at Jan 1 ---
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_MIXED_ALPHA_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Invoice 0: Capacity €50 + ExtraRec/Advance €30 + OneTime €200 = €280
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("cycle 0 - Alpha initial")
        .is_finalized_unpaid()
        .has_total(5000 + 3000 + 20000);

    // --- Cycle 1: process_cycles → Period [Feb 1, Mar 1] ---
    env.process_cycles().await;

    // --- Feb 15: Immediate plan change Alpha → Capacity Upgraded ---
    let result = env
        .services()
        .apply_plan_change_immediate_at(
            sub_id,
            TENANT_ID,
            PLAN_VERSION_MIXED_CAP_UPG_ID,
            vec![],
            feb15,
        )
        .await
        .expect("apply_plan_change_immediate_at failed");

    assert!(
        result.adjustment_invoice_id.is_some(),
        "should create adjustment invoice for capacity upgrade"
    );

    // Adjustment: only Capacity base rate differs
    // Period [Feb 1, Mar 1] = 28 days, remaining after Feb 15 = 14 days, factor = 0.5
    //   Credit Capacity:       -(5000 × 0.5) = -2500
    //   Charge Capacity:       (12000 × 0.5) = 6000
    //   Credit ExtraRec/Adv:   -(3000 × 0.5) = -1500
    //   Charge ExtraRec/Adv:    (3000 × 0.5) = 1500
    //   OneTime: 0 (both plans have it, advance_amount = 0)
    //   Rec/Arrears: 0 (is_pure_arrears, excluded)
    //   Net: -2500 + 6000 + -1500 + 1500 = 3500
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(3);
    assert_eq!(
        invoices[2].total, 3500,
        "adjustment should be 3500 cents for Capacity base upgrade proration"
    );

    // Verify subscription switched
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_MIXED_CAP_UPG_ID);

    // Verify all 4 products matched: 4 closed + 4 active = 8 components
    let all_components = env
        .get_all_subscription_components(sub_id, jan1, mar1)
        .await;
    assert_eq!(
        all_components.len(),
        8,
        "4 closed Alpha + 4 active Upgraded"
    );

    // --- Cycle 2: process_cycles → verify overage with new config ---
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(2)
        .has_period_start(mar1);

    // Invoice 3:
    // Capacity base €120 = 12000
    // Capacity overage: 600 - 500 included = 100 units × €0.02 = 200
    // ExtraRec/Advance €30 = 3000
    // Old Rec/Arrears [Feb 1, Feb 15] = 3000
    // New Rec/Arrears [Feb 15, Mar 1] = 3000
    // OneTime NOT charged
    // Total = 12000 + 200 + 3000 + 3000 + 3000 = 21200
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(4);
    assert_eq!(
        invoices[3].total, 21200,
        "cycle 2: upgraded capacity (base + overage) + advance + temporal Rec/Arrears"
    );

    // Verify overage line uses new config (100 units × €0.02 = 200¢)
    let overage_lines: Vec<_> = invoices[3]
        .line_items
        .iter()
        .filter(|l| l.metric_id.is_some())
        .collect();
    assert_eq!(overage_lines.len(), 1, "should have 1 overage line item");
    assert_eq!(
        overage_lines[0].amount_subtotal, 200,
        "overage: (600 - 500) × €0.02 = €2.00 = 200¢"
    );
}
