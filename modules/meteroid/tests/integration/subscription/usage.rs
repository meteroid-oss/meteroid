//! Subscription usage billing and cancellation tests.
//!
//! Tests for:
//! - Invoice generation with usage (arrear) components
//! - Final invoice generation at cancellation with arrear usage

use std::collections::HashMap;
use std::sync::Arc;

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::data::ids::*;
use crate::harness::{
    InvoicesAssertExt, SubscriptionAssertExt, subscription, test_env_with_seed_and_usage,
};
use crate::meteroid_it::container::SeedLevel;
use diesel_models::enums::SubscriptionStatusEnum;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageClient, MockUsageDataParams, UsageData,
};
use meteroid_store::domain::Period;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

/// Build a MockUsageClient that returns usage data for METRIC_BANDWIDTH.
fn build_usage_client(usage_units: Decimal, period_end_dates: &[NaiveDate]) -> MockUsageClient {
    let mut data = HashMap::new();
    for &end_date in period_end_dates {
        data.insert(
            MockUsageDataParams {
                metric_id: METRIC_BANDWIDTH,
                invoice_date: end_date,
            },
            UsageData {
                data: vec![GroupedUsageData {
                    value: usage_units,
                    dimensions: HashMap::new(),
                }],
                period: Period {
                    start: end_date,
                    end: end_date,
                },
            },
        );
    }
    MockUsageClient { data }
}

/// Verify that a subscription with a usage (arrear) component produces
/// invoices that include the usage charges after a billing cycle.
///
/// Plan: Rate EUR 20/mo (advance) + Bandwidth EUR 0.10/unit (arrear)
///
/// - Cycle 0 invoice: rate only (no arrear period yet)
/// - Cycle 1 invoice: rate + usage for period 0
#[tokio::test]
async fn test_usage_invoice_includes_arrear_charges() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let period2_end = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    let usage_units = Decimal::from(100); // 100 units × EUR 0.10 = EUR 10.00 = 1000 cents

    let usage_client = build_usage_client(usage_units, &[period1_end, period2_end]);
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage_client)).await;

    // Create subscription on the usage plan (Rate 20€/mo + Usage bandwidth)
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Cycle 0: advance rate only, no arrear yet
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(0);

    let rate_cents = 2000i64; // EUR 20.00/month
    assert_eq!(
        sub.mrr_cents, rate_cents,
        "Initial MRR should be the rate component"
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).has_total(rate_cents);

    // Cycle 1: renewal should produce rate + arrear usage from period 0
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active().has_cycle_index(1);
    assert_eq!(sub.mrr_cents, rate_cents, "MRR stable through renewal");

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);

    let usage_cents = 1000i64; // 100 units × 10 cents
    invoices
        .assert()
        .invoice_at(1)
        .has_total(rate_cents + usage_cents);
}

/// Cancelling a subscription with arrear usage must produce a final invoice
/// containing the usage charges for the last period.
///
/// Bug: `compute_component` (component.rs) returns early when `advance = None`
/// (terminated subscriptions have `current_period_end = None` → `is_completed = true`
/// → `advance = None`), skipping arrear lines entirely. The final invoice is never
/// created, so usage for the last period is lost.
///
/// Timeline:
///   2024-01-01  subscription start, invoice #1 (rate only)
///   2024-02-01  renewal, invoice #2 (rate + usage for period 0)
///   2024-03-01  cancellation processed → should produce invoice #3 (usage for period 1)
#[tokio::test]
async fn test_cancel_with_usage_produces_final_arrear_invoice() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let period2_end = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    let usage_units = Decimal::from(100); // 100 units × EUR 0.10 = EUR 10.00 = 1000 cents

    let usage_client = build_usage_client(usage_units, &[period1_end, period2_end]);
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage_client)).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let rate_cents = 2000i64;
    let usage_cents = 1000i64;

    // Cycle 0 → invoice #1 (rate only)
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);

    // Cycle 1 → invoice #2 (rate + arrear usage for period 0)
    env.process_cycles().await;
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(1)
        .has_total(rate_cents + usage_cents);

    // Cancel at end of period 1
    env.services()
        .cancel_subscription(
            sub_id,
            TENANT_ID,
            Some("testing final arrear invoice".to_string()),
            CancellationEffectiveAt::Date(period2_end),
            USER_ID,
        )
        .await
        .expect("cancel_subscription failed");

    // Process the cancellation
    env.process_cycles().await;

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.status, SubscriptionStatusEnum::Cancelled);

    // A third invoice must exist for the final arrear usage
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(
        invoices.len(),
        3,
        "Expected 3 invoices (initial + renewal + final arrear), got {}",
        invoices.len()
    );

    // The final invoice should contain at least the usage charges for the last period
    invoices.assert().invoice_at(2).has_total(usage_cents);

    // MRR must be exactly 0 after cancellation, not negative.
    // Bug: both `process_mrr` (via invoice finalization) and `create_churn_mrr_log`
    // (in terminate.rs) call `update_subscription_mrr_delta` with the same -2000 delta,
    // resulting in MRR = 2000 - 2000 - 2000 = -2000.
    assert_eq!(
        sub.mrr_cents, 0,
        "MRR must be exactly 0 after cancellation, got {} (double MRR removal bug)",
        sub.mrr_cents
    );
}
