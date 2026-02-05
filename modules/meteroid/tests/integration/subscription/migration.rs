//! Subscription migration tests (skip_past_invoices).
//!
//! Tests the migration mode for creating subscriptions with a past start_date
//! without generating historical invoices. Verifies:
//! - Correct subscription state (Active, correct cycle_index, period boundaries)
//! - No invoices created at import time
//! - imported_at is set
//! - Renewal works normally after migration
//! - Validation rejects invalid combinations

use chrono::{Datelike, NaiveDate};
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env};
use diesel_models::enums::{CycleActionEnum, SubscriptionStatusEnum};

// =============================================================================
// BASIC MIGRATION TESTS
// =============================================================================

/// Migration mode: past start_date + skip_past_invoices → Active, no invoices, imported_at set.
/// Start date is 2024-01-01 (default), billing_day=1, monthly.
/// The subscription should be set to the current billing period with correct cycle_index.
#[rstest]
#[tokio::test]
async fn test_migration_no_trial_is_active_without_invoices(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month, no trial
        .on_start()
        .no_trial()
        .skip_past_invoices()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;

    // Should be Active immediately
    sub.assert()
        .is_active()
        .has_pending_checkout(false)
        .is_imported();

    // cycle_index should be > 0 (start is 2024-01-01, now is 2026+)
    assert!(
        sub.cycle_index.unwrap_or(0) > 0,
        "Expected cycle_index > 0 for migrated subscription, got {:?}",
        sub.cycle_index
    );

    // Current period should contain or end on today
    let today = chrono::Utc::now().naive_utc().date();
    assert!(
        sub.current_period_start <= today,
        "Expected current_period_start ({}) <= today ({})",
        sub.current_period_start,
        today
    );
    assert!(
        sub.current_period_end.unwrap() >= today,
        "Expected current_period_end ({:?}) >= today ({})",
        sub.current_period_end,
        today
    );

    // No invoices should be created (past billing skipped)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

/// Migration: process_cycles should NOT trigger renewal while current period is ongoing.
#[rstest]
#[tokio::test]
async fn test_migration_no_premature_renewal(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let today = chrono::Utc::now().naive_utc().date();
    // Use a billing day guaranteed to be mid-period today, so process_cycles is a no-op.
    let billing_day = if today.day() == 15 { 10 } else { 15 };
    let start_date = NaiveDate::from_ymd_opt(today.year() - 1, today.month(), billing_day).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .on_start()
        .no_trial()
        .start_date(start_date)
        .skip_past_invoices()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().is_imported();
    let initial_cycle = sub.cycle_index;

    // Process cycles — since current_period_end is in the future,
    // this should NOT trigger renewal yet (no-op).
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.cycle_index, initial_cycle);

    // Still no invoices — past ones were skipped, current period not yet ended
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

/// Migration: subscription is wired up correctly for the renewal system.
/// We can't fast-forward time, but we verify all the invariants the renewal
/// worker depends on: status=Active, next_cycle_action=RenewSubscription,
/// current_period_end is set and in the future.
/// Uses a mid-period billing day to guarantee deterministic assertions.
#[rstest]
#[tokio::test]
async fn test_migration_ready_for_renewal(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let today = chrono::Utc::now().naive_utc().date();
    let billing_day = if today.day() == 15 { 10 } else { 15 };
    let start_date = NaiveDate::from_ymd_opt(2024, 1, billing_day).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .on_start()
        .no_trial()
        .start_date(start_date)
        .skip_past_invoices()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;

    // All the invariants the cycle transition worker needs
    sub.assert()
        .is_active()
        .has_next_action(Some(CycleActionEnum::RenewSubscription))
        .is_imported();

    // cycle_index must be set (> 0 for past subscriptions)
    assert!(
        sub.cycle_index.is_some() && sub.cycle_index.unwrap() > 0,
        "Expected cycle_index > 0, got {:?}",
        sub.cycle_index
    );

    // Mid-period: current_period_end must be in the future
    let period_end = sub
        .current_period_end
        .expect("current_period_end must be set");
    assert!(
        period_end > today,
        "Expected current_period_end ({}) > today ({})",
        period_end,
        today
    );

    // current_period_start must be <= today (we're in this period)
    assert!(
        sub.current_period_start <= today,
        "Expected current_period_start ({}) <= today ({})",
        sub.current_period_start,
        today
    );
}

/// Normal (non-migration) subscription should have imported_at = None.
#[rstest]
#[tokio::test]
async fn test_normal_subscription_not_imported(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().is_not_imported();
}

// =============================================================================
// MIGRATION WITH SPECIFIC START DATES
// =============================================================================

/// Migration with a start date exactly 1 year ago (12 monthly cycles).
/// Uses a mid-period billing day to avoid landing on a renewal boundary.
#[rstest]
#[tokio::test]
async fn test_migration_one_year_ago_monthly(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let today = chrono::Utc::now().naive_utc().date();
    // Fixed start date with billing_day=15. Mid-period unless today is the 15th.
    let billing_day: u32 = if today.day() == 15 { 10 } else { 15 };
    let one_year_ago =
        NaiveDate::from_ymd_opt(today.year() - 1, today.month(), billing_day).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .on_start()
        .no_trial()
        .start_date(one_year_ago)
        .skip_past_invoices()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().is_imported();

    // ~12 cycles: exactly 12 if billing_day < today.day(), 11 if billing_day > today.day().
    let cycle = sub.cycle_index.unwrap();
    let expected = if billing_day < today.day() { 12 } else { 11 };
    assert_eq!(
        cycle, expected,
        "Expected cycle_index == {} for 1-year-old monthly sub, got {}",
        expected, cycle
    );

    // current_period_end should be in the future (mid-period).
    assert!(
        sub.current_period_end.unwrap() > today,
        "Expected current_period_end > today for mid-period"
    );

    // No invoices
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

// =============================================================================
// VALIDATION TESTS
// =============================================================================

/// skip_past_invoices with future start_date should fail.
#[rstest]
#[tokio::test]
async fn test_migration_rejects_future_start_date(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let future_date = NaiveDate::from_ymd_opt(2099, 1, 1).unwrap();

    let result = env
        .services()
        .insert_subscription(
            meteroid_store::domain::CreateSubscription {
                subscription: meteroid_store::domain::SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: future_date,
                    end_date: None,
                    billing_start_date: None,
                    activation_condition:
                        meteroid_store::domain::SubscriptionActivationCondition::OnStart,
                    trial_duration: None,
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: true,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await;

    assert!(
        result.is_err(),
        "Expected error for future start_date with skip_past_invoices"
    );
}

/// skip_past_invoices with Manual activation should fail.
#[rstest]
#[tokio::test]
async fn test_migration_rejects_manual_activation(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let result = env
        .services()
        .insert_subscription(
            meteroid_store::domain::CreateSubscription {
                subscription: meteroid_store::domain::SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: None,
                    activation_condition:
                        meteroid_store::domain::SubscriptionActivationCondition::Manual,
                    trial_duration: None,
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: true,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await;

    assert!(
        result.is_err(),
        "Expected error for Manual activation with skip_past_invoices"
    );
}

/// skip_past_invoices with OnCheckout activation should fail.
#[rstest]
#[tokio::test]
async fn test_migration_rejects_on_checkout_activation(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let result = env
        .services()
        .insert_subscription(
            meteroid_store::domain::CreateSubscription {
                subscription: meteroid_store::domain::SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: None,
                    activation_condition:
                        meteroid_store::domain::SubscriptionActivationCondition::OnCheckout,
                    trial_duration: None,
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: true,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await;

    assert!(
        result.is_err(),
        "Expected error for OnCheckout activation with skip_past_invoices"
    );
}

// =============================================================================
// MIGRATION WITH FREE TRIAL (TRIAL ALREADY ENDED)
// =============================================================================

/// Migration with a free trial plan where the trial has already ended.
/// start_date = 2024-01-01, trial = 14 days → trial ended 2024-01-15.
/// Since now >> 2024-01-15, the subscription should be Active (post-trial).
#[rstest]
#[tokio::test]
async fn test_migration_free_trial_already_ended(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // Free trial plan
        .on_start()
        .trial_days(14)
        .skip_past_invoices()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;

    // Trial ended long ago, subscription should be Active
    sub.assert()
        .is_active()
        .is_imported()
        .has_trial_duration(Some(14));

    // cycle_index > 0 (months since trial ended on 2024-01-15)
    assert!(
        sub.cycle_index.unwrap_or(0) > 0,
        "Expected cycle_index > 0 for post-trial migration"
    );

    // No invoices
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

// =============================================================================
// MIGRATION WITH TRIAL STILL ACTIVE
// =============================================================================

/// Migration with a free trial that is still ongoing.
/// start_date = 3 days ago, trial = 30 days → trial ends in 27 days.
/// The subscription should be TrialActive with cycle_index=0.
#[rstest]
#[tokio::test]
async fn test_migration_free_trial_still_active(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let today = chrono::Utc::now().naive_utc().date();
    let three_days_ago = today - chrono::Duration::days(3);

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // Paid plan with free trial
        .on_start()
        .start_date(three_days_ago)
        .trial_days(30) // 30-day trial, only 3 days elapsed
        .skip_past_invoices()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;

    // Trial is still active
    sub.assert()
        .is_trial_active()
        .is_imported()
        .has_trial_duration(Some(30))
        .has_next_action(Some(CycleActionEnum::EndTrial));

    // cycle_index should be 0 during trial
    assert_eq!(
        sub.cycle_index,
        Some(0),
        "Expected cycle_index=0 during active trial"
    );

    // current_period should span the trial duration
    assert_eq!(
        sub.current_period_start, three_days_ago,
        "Expected current_period_start to match start_date"
    );
    let expected_trial_end = three_days_ago + chrono::Duration::days(30);
    assert_eq!(
        sub.current_period_end,
        Some(expected_trial_end),
        "Expected current_period_end to be start_date + 30 days"
    );

    // No invoices (free trial, no billing yet)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

// =============================================================================
// MIGRATION WITH PAID TRIAL
// =============================================================================

/// Migration with a paid trial plan where the trial has already ended.
/// Paid trials bill immediately but provide trial features.
/// start_date = 2024-01-01, trial = 14 days → trial ended 2024-01-15, billing started 2024-01-01.
/// The subscription should be Active with cycle_index calculated from billing start (not trial end).
#[rstest]
#[tokio::test]
async fn test_migration_paid_trial_already_ended(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID) // Paid plan with paid trial (trial_is_free = false)
        .on_start()
        .trial_days(14)
        .skip_past_invoices()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;

    // Trial ended, subscription should be Active
    sub.assert()
        .is_active()
        .is_imported()
        .has_trial_duration(Some(14));

    // For paid trials, billing starts immediately (start_date=2024-01-01).
    // cycle_index should be calculated from start_date, not trial end.
    // Since now is 2026+, we expect cycle_index > 20 (24+ months elapsed).
    let cycle = sub.cycle_index.unwrap();
    assert!(
        cycle > 20,
        "Expected cycle_index > 20 for paid trial migration (billing from start_date), got {}",
        cycle
    );

    // No invoices
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

// =============================================================================
// RENEWAL BOUNDARY TESTS
// =============================================================================

/// Migration with start_date chosen so that today is exactly a renewal boundary.
/// The current period should END today (not start today), so the automation
/// picks it up and creates the invoice for the completed period.
#[rstest]
#[tokio::test]
async fn test_migration_renewal_boundary_uses_ending_period(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let today = chrono::Utc::now().naive_utc().date();
    // Pick a start_date with the same day-of-month as today, 1 year ago.
    // This means billing_day_anchor = today.day(), so today is a renewal boundary.
    let start_date = NaiveDate::from_ymd_opt(today.year() - 1, today.month(), today.day()).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month, no trial
        .on_start()
        .no_trial()
        .start_date(start_date)
        .skip_past_invoices()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().is_imported();

    // On a renewal boundary, current_period_end should be today (the ending period).
    assert_eq!(
        sub.current_period_end,
        Some(today),
        "Expected current_period_end == today ({}) on a renewal boundary, got {:?}",
        today,
        sub.current_period_end
    );

    // current_period_start should be one billing period before today.
    assert!(
        sub.current_period_start < today,
        "Expected current_period_start ({}) < today ({})",
        sub.current_period_start,
        today
    );

    // No invoices yet (past billing skipped, automation hasn't run).
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Running process_cycles should pick it up (current_period_end <= today)
    // and create an invoice for the completed period.
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);

    // After renewal, subscription should have advanced to the next period.
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();
    assert_eq!(
        sub.current_period_start, today,
        "After renewal, current_period_start should be today"
    );
    assert!(
        sub.current_period_end.unwrap() > today,
        "After renewal, current_period_end should be in the future"
    );
}

/// Migration with start_date chosen so that today is NOT a renewal boundary (mid-period).
/// The current period should span across today, and process_cycles should be a no-op.
#[rstest]
#[tokio::test]
async fn test_migration_mid_period_no_renewal(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let today = chrono::Utc::now().naive_utc().date();
    // Pick a billing day that is NOT today's day-of-month, ensuring mid-period.
    let billing_day = if today.day() == 15 { 10 } else { 15 };
    let start_date = NaiveDate::from_ymd_opt(today.year() - 1, today.month(), billing_day).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month, no trial
        .on_start()
        .no_trial()
        .start_date(start_date)
        .skip_past_invoices()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().is_imported();

    // Mid-period: current_period_end should be in the future.
    assert!(
        sub.current_period_end.unwrap() > today,
        "Expected current_period_end ({:?}) > today ({}) for mid-period",
        sub.current_period_end,
        today
    );
    assert!(
        sub.current_period_start <= today,
        "Expected current_period_start ({}) <= today ({})",
        sub.current_period_start,
        today
    );

    // No invoices.
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // process_cycles should NOT trigger renewal (period ongoing).
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // Subscription should be unchanged.
    let sub_after = env.get_subscription(sub_id).await;
    assert_eq!(sub_after.cycle_index, sub.cycle_index);
}

/// Migration with a paid trial that is still ongoing.
/// Paid trials bill immediately, so even during trial we should be billing.
/// start_date = 3 days ago, trial = 30 days.
/// The subscription should be TrialActive but with RenewSubscription action (paid trial bills).
#[rstest]
#[tokio::test]
async fn test_migration_paid_trial_still_active(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let today = chrono::Utc::now().naive_utc().date();
    let three_days_ago = today - chrono::Duration::days(3);

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID) // Paid plan with paid trial
        .on_start()
        .start_date(three_days_ago)
        .trial_days(30)
        .skip_past_invoices()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;

    // Trial is still active, but for paid trial billing happens immediately
    sub.assert()
        .is_trial_active()
        .is_imported()
        .has_trial_duration(Some(30));

    // cycle_index should be 0 (we're in the first billing period)
    assert_eq!(
        sub.cycle_index,
        Some(0),
        "Expected cycle_index=0 during active paid trial"
    );

    // No invoices (skip_past_invoices skips the initial billing too)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();
}

// =============================================================================
// MIGRATION WITH END DATE IN THE PAST
// =============================================================================

/// Migration with end_date before today.
/// The subscription is created Active (migration doesn't handle end_date specially),
/// then process_cycles should detect end_date and transition it through to Completed.
#[rstest]
#[tokio::test]
async fn test_migration_end_date_before_today(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let today = chrono::Utc::now().naive_utc().date();
    // Start 1 year ago on the same day-of-month as today (renewal boundary).
    let start_date = NaiveDate::from_ymd_opt(today.year() - 1, today.month(), today.day()).unwrap();
    // End date is 2 months ago (well before today).
    let end_date = today - chrono::Duration::days(60);

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .on_start()
        .no_trial()
        .start_date(start_date)
        .end_date(end_date)
        .skip_past_invoices()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    // Initially created as Active (migration path doesn't handle end_date directly).
    sub.assert().is_active().is_imported();

    // No invoices yet.
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().assert_empty();

    // First process_cycles: renews and detects end_date <= new_period_end,
    // sets next_cycle_action = EndSubscription with period truncated to end_date.
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .has_next_action(Some(CycleActionEnum::EndSubscription));
    assert_eq!(
        sub.current_period_end,
        Some(end_date),
        "Period should be truncated to end_date"
    );

    // Second process_cycles: executes EndSubscription → Completed.
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert()
        .has_status(SubscriptionStatusEnum::Completed)
        .has_next_action(None);
}
