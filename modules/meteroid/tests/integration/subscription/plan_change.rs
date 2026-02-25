//! Plan change integration tests.
//!
//! Tests for scheduling, previewing, canceling, and executing plan changes.
//! Uses product-backed plans (Starter & Pro) where components match by product_id.

use chrono::NaiveDate;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env};

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
    // During RenewSubscription, the code detects the pending ApplyPlanChange event
    // at the period boundary. It applies the plan change (swapping components to
    // Pro pricing), then continues with the renewal — advancing the period and
    // billing at the new Pro prices atomically.
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

    // Components should reference Pro products
    let components = env.get_subscription_components(sub_id).await;
    assert_eq!(components.len(), 2, "should have 2 components on Pro plan");
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

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // First schedule succeeds
    let first_event = env
        .services()
        .schedule_plan_change(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![])
        .await
        .expect("first schedule should succeed");

    // Second schedule should also succeed, replacing the first
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

// =============================================================================
// IMMEDIATE PLAN CHANGE
// =============================================================================

/// Immediate upgrade: Starter→Pro creates adjustment invoice with prorated amounts.
///
/// Starter: Platform Fee €29 + Seats €10×1 = €39/mo (3900 cents)
/// Pro:     Platform Fee €99 + Seats €25×1 = €124/mo (12400 cents)
///
/// On a 31-day month starting Jan 1, with change on Jan 1 (day 0),
/// proration factor = 31/31 = 1.0.
/// Credits: -(3900 * 1.0) = -3900
/// Charges: +(12400 * 1.0) = +12400
/// Net: +8500
#[rstest]
#[tokio::test]
async fn test_immediate_plan_change_upgrade(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Use today so the billing period [today, today+1month] encompasses the change_date.
    let today = chrono::Utc::now().naive_utc().date();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(today)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .is_active()
        .has_cycle_index(0)
        .has_period_start(today);

    // Apply immediate upgrade
    let result = env
        .services()
        .apply_plan_change_immediate(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![], false)
        .await
        .expect("apply_plan_change_immediate failed");

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

    // Components should be Pro components
    let components = env.get_subscription_components(sub_id).await;
    assert_eq!(components.len(), 2, "should have 2 Pro components");

    // Should have: initial Starter invoice + adjustment invoice
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);

    // Adjustment invoice should be finalized
    invoices
        .assert()
        .invoice_at(1)
        .with_context("adjustment invoice")
        .has_status(meteroid_store::domain::enums::InvoiceStatusEnum::Finalized)
        .check_prorated(true);

    // Net should be positive (upgrade: charges > credits)
    // change_date = period_start → factor = 1.0
    // Credits: -(2900 + 1000) = -3900, Charges: +(9900 + 2500) = +12400, Net: +8500
    let adj = &invoices[1];
    assert!(adj.total > 0, "adjustment total should be positive for upgrade");
    assert_eq!(adj.total, 8500, "adjustment total should be 8500 (full period proration)");
}

/// Immediate downgrade should be rejected.
#[rstest]
#[tokio::test]
async fn test_immediate_plan_change_rejects_downgrade(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let today = chrono::Utc::now().naive_utc().date();

    // Start on the more expensive Pro plan
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PRO_ID)
        .start_date(today)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();

    // Downgrading to Starter should fail
    let result = env
        .services()
        .apply_plan_change_immediate(sub_id, TENANT_ID, PLAN_VERSION_STARTER_ID, vec![], false)
        .await;

    assert!(
        result.is_err(),
        "immediate downgrade should be rejected"
    );

    // Subscription should remain on Pro
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_PRO_ID);
}

/// Immediate change cancels any pending scheduled plan change.
#[rstest]
#[tokio::test]
async fn test_immediate_plan_change_cancels_pending_scheduled(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let today = chrono::Utc::now().naive_utc().date();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(today)
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
        .apply_plan_change_immediate(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![], false)
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
    // change_date = period_start = today → factor = 1.0
    let proration = result.proration.expect("proration should be present for Immediate mode");
    assert!(proration.charges_total_cents > 0, "charges should be positive");
    assert!(proration.credits_total_cents < 0, "credits should be negative");
    assert!(
        proration.net_amount_cents > 0,
        "net should be positive for upgrade"
    );
    assert!(proration.days_remaining > 0, "days_remaining should be > 0");
    assert!(proration.days_in_period > 0, "days_in_period should be > 0");
}

/// End-of-period plan change still works with temporal rotation.
/// Verifies no regression from the new temporal component model.
#[rstest]
#[tokio::test]
async fn test_end_of_period_change_with_temporal_rotation(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Get initial component count
    let components = env.get_subscription_components(sub_id).await;
    assert_eq!(components.len(), 2, "Starter should have 2 components");

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

    // Active components should be Pro components only (old ones closed)
    let components = env.get_subscription_components(sub_id).await;
    assert_eq!(components.len(), 2, "should have 2 active Pro components");

    // All active components should have effective_from set
    for comp in &components {
        assert!(
            comp.effective_to.is_none(),
            "active components should not have effective_to set"
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
