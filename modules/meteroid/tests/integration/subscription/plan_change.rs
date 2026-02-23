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

    let preview = env
        .services()
        .preview_plan_change(sub_id, TENANT_ID, PLAN_VERSION_PRO_ID, vec![])
        .await
        .expect("preview_plan_change failed");

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
