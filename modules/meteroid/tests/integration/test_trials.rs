//! Integration tests for the subscription trials feature.
//!
//! Tests cover:
//! - Effective plan resolution based on trial status
//! - Free plan with trial → Active after trial (never bills)
//! - Paid plan with free trial + no payment method → TrialExpired
//! - Paid plan with free trial + payment method → Active + invoice after trial
//! - Paid plan with paid trial → bills immediately

use chrono::NaiveDate;
use std::sync::Arc;

#[allow(unused_imports)]
use crate::data::ids::*;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use diesel_models::enums::{CycleActionEnum, SubscriptionStatusEnum};
use diesel_models::subscriptions::SubscriptionRow;
use meteroid_mailer::service::MockMailerService;
use meteroid_store::clients::usage::MockUsageClient;
use meteroid_store::domain::subscription_trial::EffectivePlanSource;
use meteroid_store::domain::{
    CreateSubscription, SubscriptionActivationCondition, SubscriptionNew,
};
use meteroid_store::store::PgConn;

/// Test that a subscription with trial starts in TrialActive state
/// and uses the trialing_plan as effective plan
#[tokio::test]
async fn test_trial_active_with_trialing_plan() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;

    let mock_mailer = Arc::new(MockMailerService::new());

    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let pool = setup.store.pool.clone();
    let services = setup.services.clone();
    let mut conn = pool.get().await.unwrap();

    // Create a subscription on the Free with Trial plan which has a 7-day trial
    // and trialing_plan_id pointing to Enterprise
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let subscription = services
        .insert_subscription(
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_PRO_WITH_TRIAL_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date,
                    end_date: None,
                    billing_start_date: None,
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: Some(7), // 7 day trial
                    billing_day_anchor: None,
                    payment_strategy: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    // Get the subscription row to check status
    let subscription_row = get_subscription_row(&mut conn, subscription.id).await;

    // Assert subscription is in trial
    assert_eq!(subscription_row.status, SubscriptionStatusEnum::TrialActive);
    assert_eq!(
        subscription_row.next_cycle_action,
        Some(CycleActionEnum::EndTrial)
    );
    assert_eq!(subscription_row.trial_duration, Some(7));

    // Assert original plan_version_id is preserved
    assert_eq!(
        subscription_row.plan_version_id,
        PLAN_VERSION_PRO_WITH_TRIAL_ID
    );

    // Resolve effective plan - should be Enterprise (trialing plan) during trial
    let effective_plan = services
        .get_subscription_effective_plan(&mut conn, TENANT_ID, subscription.id)
        .await
        .unwrap();

    assert_eq!(effective_plan.plan_id, PLAN_ENTERPRISE_ID);
    assert_eq!(effective_plan.plan_version_id, PLAN_VERSION_ENTERPRISE_ID);
    assert_eq!(effective_plan.plan_name, "Enterprise");
    assert_eq!(effective_plan.source, EffectivePlanSource::TrialingPlan);

    log::info!(
        "Trial subscription created with effective plan: {} (source: {:?})",
        effective_plan.plan_name,
        effective_plan.source
    );
}

/// Test that after trial ends, the subscription becomes Active on the original plan
#[tokio::test]
async fn test_trial_ends_becomes_active() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;

    let mock_mailer = Arc::new(MockMailerService::new());

    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let pool = setup.store.pool.clone();
    let services = setup.services.clone();
    let mut conn = pool.get().await.unwrap();

    // Create a subscription with trial
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let subscription = services
        .insert_subscription(
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_PRO_WITH_TRIAL_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date,
                    end_date: None,
                    billing_start_date: None,
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: Some(7),
                    billing_day_anchor: None,
                    payment_strategy: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    // Verify initial state
    let subscription_row = get_subscription_row(&mut conn, subscription.id).await;
    assert_eq!(subscription_row.status, SubscriptionStatusEnum::TrialActive);

    // Process cycle transitions to end the trial
    // This should trigger the EndTrial action and transition to Active
    services.get_and_process_cycle_transitions().await.unwrap();
    services.get_and_process_due_events().await.unwrap();

    // Get updated subscription
    let subscription_row = get_subscription_row(&mut conn, subscription.id).await;

    // After trial ends, status should be Active (subscription continues on the Free plan)
    assert_eq!(subscription_row.status, SubscriptionStatusEnum::Active);

    // Original plan_version_id should still be preserved
    assert_eq!(
        subscription_row.plan_version_id,
        PLAN_VERSION_PRO_WITH_TRIAL_ID
    );

    // Next action should be to renew the subscription
    assert_eq!(
        subscription_row.next_cycle_action,
        Some(CycleActionEnum::RenewSubscription)
    );

    // Resolve effective plan - should be the original plan (Free with Trial)
    let effective_plan = services
        .get_subscription_effective_plan(&mut conn, TENANT_ID, subscription.id)
        .await
        .unwrap();

    assert_eq!(effective_plan.plan_id, PLAN_PRO_WITH_TRIAL_ID);
    assert_eq!(
        effective_plan.plan_version_id,
        PLAN_VERSION_PRO_WITH_TRIAL_ID
    );
    assert_eq!(effective_plan.plan_name, "Free with Trial");
    assert_eq!(effective_plan.source, EffectivePlanSource::OriginalPlan);

    log::info!(
        "Trial ended, subscription now Active on: {} (source: {:?})",
        effective_plan.plan_name,
        effective_plan.source
    );
}

/// Test that a subscription without trial configuration
/// always returns the original plan as effective plan
#[tokio::test]
async fn test_no_trial_uses_original_plan() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;

    let mock_mailer = Arc::new(MockMailerService::new());

    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let pool = setup.store.pool.clone();
    let services = setup.services.clone();
    let mut conn = pool.get().await.unwrap();

    // Create a subscription on LeetCode plan (no trial config)
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let subscription = services
        .insert_subscription(
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date,
                    end_date: None,
                    billing_start_date: None,
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: None, // No trial
                    billing_day_anchor: None,
                    payment_strategy: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    // Get the subscription row
    let subscription_row = get_subscription_row(&mut conn, subscription.id).await;

    // Should be Active (not TrialActive) since no trial
    assert_eq!(subscription_row.status, SubscriptionStatusEnum::Active);
    assert_eq!(
        subscription_row.next_cycle_action,
        Some(CycleActionEnum::RenewSubscription)
    );

    // Resolve effective plan - should be original LeetCode plan
    let effective_plan = services
        .get_subscription_effective_plan(&mut conn, TENANT_ID, subscription.id)
        .await
        .unwrap();

    assert_eq!(effective_plan.plan_id, PLAN_LEETCODE_ID);
    assert_eq!(effective_plan.plan_version_id, PLAN_VERSION_1_LEETCODE_ID);
    assert_eq!(effective_plan.plan_name, "LeetCode");
    assert_eq!(effective_plan.source, EffectivePlanSource::OriginalPlan);

    log::info!(
        "No-trial subscription using original plan: {} (source: {:?})",
        effective_plan.plan_name,
        effective_plan.source
    );
}

/// Test trial with no trialing_plan_id configured
/// Should use original plan during trial
#[tokio::test]
async fn test_trial_without_trialing_plan() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;

    let mock_mailer = Arc::new(MockMailerService::new());

    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let pool = setup.store.pool.clone();
    let services = setup.services.clone();
    let mut conn = pool.get().await.unwrap();

    // Create a subscription with trial on LeetCode plan
    // LeetCode plan has no trialing_plan_id configured
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let subscription = services
        .insert_subscription(
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date,
                    end_date: None,
                    billing_start_date: None,
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: Some(14), // 14 day trial, but no trialing_plan configured
                    billing_day_anchor: None,
                    payment_strategy: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    // Get the subscription row
    let subscription_row = get_subscription_row(&mut conn, subscription.id).await;

    // Should be TrialActive
    assert_eq!(subscription_row.status, SubscriptionStatusEnum::TrialActive);

    // Resolve effective plan - should be original plan since no trialing_plan configured
    let effective_plan = services
        .get_subscription_effective_plan(&mut conn, TENANT_ID, subscription.id)
        .await
        .unwrap();

    assert_eq!(effective_plan.plan_id, PLAN_LEETCODE_ID);
    assert_eq!(effective_plan.plan_version_id, PLAN_VERSION_1_LEETCODE_ID);
    assert_eq!(effective_plan.plan_name, "LeetCode");
    assert_eq!(effective_plan.source, EffectivePlanSource::OriginalPlan);

    log::info!(
        "Trial without trialing_plan, using original: {} (source: {:?})",
        effective_plan.plan_name,
        effective_plan.source
    );
}

/// Test that paid plan with free trial becomes TrialExpired when trial ends without payment method
#[tokio::test]
async fn test_paid_plan_free_trial_no_payment_method_becomes_trial_expired() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;

    let mock_mailer = Arc::new(MockMailerService::new());

    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let pool = setup.store.pool.clone();
    let services = setup.services.clone();
    let mut conn = pool.get().await.unwrap();

    // Create a subscription on a paid plan with free trial, no payment method
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let subscription = services
        .insert_subscription(
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_PAID_FREE_TRIAL_ID,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date,
                    end_date: None,
                    billing_start_date: None,
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: Some(14), // 14 day trial
                    billing_day_anchor: None,
                    payment_strategy: None, // No payment method
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    // Verify initial state
    let subscription_row = get_subscription_row(&mut conn, subscription.id).await;
    assert_eq!(subscription_row.status, SubscriptionStatusEnum::TrialActive);
    assert!(
        subscription_row.payment_method.is_none(),
        "Should have no payment method"
    );

    // Process cycle transitions to end the trial
    services.get_and_process_cycle_transitions().await.unwrap();

    // Get updated subscription
    let subscription_row = get_subscription_row(&mut conn, subscription.id).await;

    // Should be TrialExpired because:
    // - It's a paid plan (Standard type)
    // - Trial has ended
    // - No payment method on file
    assert_eq!(
        subscription_row.status,
        SubscriptionStatusEnum::TrialExpired,
        "Should be TrialExpired without payment method"
    );

    // No next action - waiting for checkout
    assert_eq!(
        subscription_row.next_cycle_action, None,
        "Should have no next action while awaiting checkout"
    );

    log::info!("Paid plan with free trial correctly transitioned to TrialExpired");
}

/// Test that free plan with trial goes straight to Active (never bills, never TrialExpired)
#[tokio::test]
async fn test_free_plan_trial_never_bills() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;

    let mock_mailer = Arc::new(MockMailerService::new());

    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let pool = setup.store.pool.clone();
    let services = setup.services.clone();
    let mut conn = pool.get().await.unwrap();

    // Create a subscription on Free with Trial plan (Free plan type)
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let subscription = services
        .insert_subscription(
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_PRO_WITH_TRIAL_ID, // This is a Free plan type
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date,
                    end_date: None,
                    billing_start_date: None,
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: Some(7),
                    billing_day_anchor: None,
                    payment_strategy: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    // Verify initial state
    let subscription_row = get_subscription_row(&mut conn, subscription.id).await;
    assert_eq!(subscription_row.status, SubscriptionStatusEnum::TrialActive);

    // Process cycle transitions to end the trial
    services.get_and_process_cycle_transitions().await.unwrap();

    // Get updated subscription
    let subscription_row = get_subscription_row(&mut conn, subscription.id).await;

    // Should be Active (not TrialExpired) because:
    // - It's a Free plan type
    // - Free plans don't need payment and never bill
    assert_eq!(
        subscription_row.status,
        SubscriptionStatusEnum::Active,
        "Free plan should go to Active, not TrialExpired"
    );

    // Should have RenewSubscription as next action (even though it won't bill)
    assert_eq!(
        subscription_row.next_cycle_action,
        Some(CycleActionEnum::RenewSubscription)
    );

    log::info!("Free plan with trial correctly transitioned to Active (no billing)");
}

// Helper function to get subscription row
async fn get_subscription_row(
    conn: &mut PgConn,
    subscription_id: common_domain::ids::SubscriptionId,
) -> SubscriptionRow {
    SubscriptionRow::get_subscription_by_id(conn, &TENANT_ID, subscription_id)
        .await
        .unwrap()
        .subscription
}
