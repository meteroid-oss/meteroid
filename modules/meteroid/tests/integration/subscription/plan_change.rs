//! Plan change integration tests.
//!
//! Tests for scheduling, previewing, canceling, and executing plan changes.
//! Uses product-backed plans (Starter & Pro) where components match by product_id.

use chrono::{NaiveDate, NaiveTime};
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env};
use meteroid_store::domain::SlotUpgradeBillingMode;
use meteroid_store::domain::subscription_components::{
    ComponentParameterization, ComponentParameters,
};
use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;

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
        .apply_plan_change_immediate(sub_id, TENANT_ID, PLAN_VERSION_STARTER_ID, vec![], false)
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
/// Uses a fixed start date 15 days before a known period_end to test actual proration math,
/// not just the trivial factor=1.0 case.
///
/// Starter: Platform Fee €29 + Seats €10×1 = €39/mo (3900 cents)
/// Pro:     Platform Fee €99 + Seats €25×1 = €124/mo (12400 cents)
#[rstest]
#[tokio::test]
async fn test_immediate_plan_change_upgrade(#[future] test_env: TestEnv) {
    let env = test_env.await;

    // Use a fixed start date 15 days in the past to get a non-trivial proration factor
    let today = chrono::Utc::now().naive_utc().date();
    let start_date = today - chrono::Duration::days(15);

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(0);

    // Read actual period to compute expected proration deterministically
    let period_start = sub.current_period_start;
    let period_end = sub.current_period_end.unwrap();
    let days_in_period = (period_end - period_start).num_days() as f64;
    let days_remaining = (period_end - today).num_days() as f64;
    let factor = days_remaining / days_in_period;

    // Verify we have a non-trivial factor (not 1.0 or 0.0)
    assert!(
        factor > 0.1 && factor < 0.99,
        "factor should be mid-period (got {factor:.4})"
    );

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

    // Verify prorated amounts match expected calculation
    // Credits: -(3900 * factor), Charges: +(12400 * factor), Net: +8500 * factor
    let credit_platform = -((2900_f64 * factor).round() as i64);
    let credit_seats = -((1000_f64 * factor).round() as i64);
    let charge_platform = (9900_f64 * factor).round() as i64;
    let charge_seats = (2500_f64 * factor).round() as i64;
    let expected_net = credit_platform + credit_seats + charge_platform + charge_seats;

    let adj = &invoices[1];
    assert!(adj.total > 0, "adjustment total should be positive for upgrade");
    assert_eq!(
        adj.total, expected_net,
        "adjustment total should match prorated Starter→Pro (factor={factor:.4})"
    );
}

/// Immediate downgrade should be rejected.
#[rstest]
#[tokio::test]
async fn test_immediate_plan_change_rejects_downgrade(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let start_date = NaiveDate::from_ymd_opt(2024, 6, 1).unwrap();

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

    // Downgrading to Starter should fail
    let result = env
        .services()
        .apply_plan_change_immediate(sub_id, TENANT_ID, PLAN_VERSION_STARTER_ID, vec![], false)
        .await;

    assert!(result.is_err(), "immediate downgrade should be rejected");

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
        .get_active_slots_value(TENANT_ID, sub_id, "seat".to_string(), None)
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
        .get_active_slots_value(TENANT_ID, sub_id, "seat".to_string(), None)
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
            "seat".to_string(),
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
/// Starter: Platform Fee €29 + Seats €10×5 = €79/mo (7900 cents)
/// Pro:     Platform Fee €99 + Seats €25×5 = €224/mo (22400 cents)
///
/// Uses fixed start date 10 days in the past for deterministic proration.
#[rstest]
#[tokio::test]
async fn test_immediate_plan_change_preserves_slot_count(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let today = chrono::Utc::now().naive_utc().date();
    let start_date = today - chrono::Duration::days(10);

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
        .get_active_slots_value(TENANT_ID, sub_id, "seat".to_string(), None)
        .await
        .expect("get slots");
    assert_eq!(slots, 5, "should have 5 seats");

    // Read actual period for deterministic proration
    let sub = env.get_subscription(sub_id).await;
    let period_start = sub.current_period_start;
    let period_end = sub.current_period_end.unwrap();
    let days_in_period = (period_end - period_start).num_days() as f64;
    let days_remaining = (period_end - today).num_days() as f64;
    let factor = days_remaining / days_in_period;

    // --- Immediate upgrade to Pro ---
    let result = env
        .services()
        .apply_plan_change_immediate(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![], false)
        .await
        .expect("immediate plan change failed");

    assert!(
        result.adjustment_invoice_id.is_some(),
        "should create adjustment invoice"
    );

    // --- Verify slot count still 5 after plan change ---
    let slots = env
        .store()
        .get_active_slots_value(TENANT_ID, sub_id, "seat".to_string(), None)
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
    // Starter with 5 seats: 2900 + 5*1000 = 7900
    // Pro with 5 seats: 9900 + 5*2500 = 22400
    let credit_platform = -((2900_f64 * factor).round() as i64);
    let credit_seats = -((5000_f64 * factor).round() as i64);
    let charge_platform = (9900_f64 * factor).round() as i64;
    let charge_seats = (12500_f64 * factor).round() as i64;
    let expected_net = credit_platform + credit_seats + charge_platform + charge_seats;

    let invoices = env.get_invoices(sub_id).await;
    let adj = &invoices[invoices.len() - 1];
    assert_eq!(
        adj.total, expected_net,
        "adjustment should reflect 5-seat proration (factor={factor:.4})"
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
            "seat".to_string(),
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
/// Uses start_date = today - 14 days so the current period encompasses today.
/// Reads the actual period from the subscription to compute expected proration deterministically.
///
/// LeetCode: €35/mo (rate only) → Starter: €29/mo platform fee + €10/seat
/// All components are added/removed (no product match), so:
///   Credit: -(3500 × factor)
///   Charge: +(2900 × factor) + (5000 × factor)
#[rstest]
#[tokio::test]
async fn test_immediate_plan_change_rate_only_to_plan_with_slots(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let today = chrono::Utc::now().naive_utc().date();
    let start_date = today - chrono::Duration::days(14);

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

    // 3. Read actual period to compute expected proration deterministically
    let sub = env.get_subscription(sub_id).await;
    let period_start = sub.current_period_start;
    let period_end = sub.current_period_end.unwrap();
    let days_in_period = (period_end - period_start).num_days() as f64;
    let days_remaining = (period_end - today).num_days() as f64;
    let factor = days_remaining / days_in_period;

    // 4. Immediate plan change to Starter with initial_slot_count = 5
    let result = env
        .services()
        .apply_plan_change_immediate(
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
            false,
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
        .get_active_slots_value(TENANT_ID, sub_id, "seat".to_string(), None)
        .await
        .expect("get slots after plan change");
    assert_eq!(slots, 5, "slot count should be 5 from initial_slot_count");

    // 6. Verify adjustment invoice proration
    //    Credit: LeetCode Rate -(3500 × factor)
    //    Charge: Starter Platform Fee +(2900 × factor)
    //    Charge: Starter Seats 5×€10 +(5000 × factor)
    let credit = -((3500_f64 * factor).round() as i64);
    let charge_platform = (2900_f64 * factor).round() as i64;
    let charge_seats = (5000_f64 * factor).round() as i64;
    let expected_net = credit + charge_platform + charge_seats;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    assert_eq!(
        invoices[1].total, expected_net,
        "adjustment total should match prorated rate-only → 5-seat Starter (factor={factor:.4}, \
         credit={credit}, platform={charge_platform}, seats={charge_seats})"
    );

    // 7. Verify subscription is now on Starter
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_STARTER_ID);
}
