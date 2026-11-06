//! Subscription usage billing and cancellation tests.
//!
//! Tests for:
//! - Invoice generation with usage (arrear) components
//! - Final invoice generation at cancellation with arrear usage

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{NaiveDate, NaiveTime};
use rust_decimal::Decimal;

use common_domain::ids::*;
use uuid::uuid;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{
    InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env_with_seed_and_usage,
};
use crate::meteroid_it::container::SeedLevel;
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use diesel_models::enums::SubscriptionStatusEnum;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageClient, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::enums::{FeeTypeEnum, InvoiceStatusEnum, InvoiceType};
use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
};
use meteroid_store::domain::subscription_amendment::{
    AddOnChanges, ComponentChanges, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::domain::{BillingPeriodEnum, Invoice, UsagePeriod, UsagePricingModel};
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

/// Build a MockUsageClient that returns usage data for METRIC_BANDWIDTH.
fn build_usage_client(usage_units: Decimal, periods: &[(NaiveDate, NaiveDate)]) -> MockUsageClient {
    let mut data = HashMap::new();
    for &(start, end) in periods {
        data.insert(
            MockUsageDataParams {
                metric_id: METRIC_BANDWIDTH,
                period_start: start.and_time(NaiveTime::MIN),
                period_end: end.and_time(NaiveTime::MIN),
            },
            UsageData {
                data: vec![GroupedUsageData {
                    value: usage_units,
                    dimensions: HashMap::new(),
                }],
                period: UsagePeriod {
                    start: start.and_time(NaiveTime::MIN),
                    end: end.and_time(NaiveTime::MIN),
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

    let usage_client = build_usage_client(
        usage_units,
        &[(start_date, period1_end), (period1_end, period2_end)],
    );
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

    let usage_client = build_usage_client(
        usage_units,
        &[(start_date, period1_end), (period1_end, period2_end)],
    );
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
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            Some("testing final arrear invoice".to_string()),
            CancellationEffectiveAt::Date(period2_end),
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

// A purely-usage plan version (single usage component, no advance/recurring line) seeded inline.
// With this plan the renewal invoice is empty whenever no usage has been imported for the
// just-closed period — the exact shape of the customer that only imports events during the grace
// period.
pub(super) const PLAN_VERSION_USAGE_ONLY_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("019438e0-0f01-7000-8000-0000000000f1"));
const COMP_USAGE_ONLY_API_CALLS_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0f02-7000-8000-0000000000f2"));
pub(super) const PRICE_USAGE_ONLY_API_CALLS_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0f03-7000-8000-0000000000f3"));

pub(super) async fn seed_usage_only_plan(env: &TestEnv) {
    let mut conn = env.conn().await;
    PlanSeed::seed_version(
        &mut conn,
        PLAN_USAGE_ALPHA_ID,
        PLAN_VERSION_USAGE_ONLY_ID,
        90,
        "EUR",
        &[SeedComp::usage(
            COMP_USAGE_ONLY_API_CALLS_ID,
            "API Calls",
            PRODUCT_API_CALLS_ID,
            METRIC_BANDWIDTH,
            PRICE_USAGE_ONLY_API_CALLS_ID,
            DieselBillingPeriodEnum::Monthly,
            UsagePricingModel::PerUnit {
                rate: Decimal::new(10, 2), // EUR 0.10 / unit
            },
        )],
    )
    .await
    .expect("failed to seed usage-only plan version");
}

/// A usage-only subscription whose renewal period has *no imported usage yet* must still produce
/// an invoice: an empty draft is created as the finalization anchor and, still empty at
/// finalization, is Closed (never numbered) rather than being silently skipped.
///
/// Regression: previously `create_subscription_draft_invoice` returned `Ok(None)` on empty lines,
/// so no draft and no finalization were ever scheduled. Usage imported later during the grace
/// period had nothing to attach to and was never billed.
#[tokio::test]
async fn test_usage_only_renewal_with_no_usage_is_closed_not_skipped() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // noop usage client -> zero usage for every period.
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    seed_usage_only_plan(&env).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Cycle 0 (advance) has nothing to bill up front and no arrears period yet -> no invoice.
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(
        invoices.len(),
        0,
        "a purely-usage plan bills nothing on its initial (cycle 0) invoice"
    );

    // Renewal bills the just-closed period, which still has zero usage.
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(
        invoices.len(),
        1,
        "the renewal draft is created even when no usage has been imported yet"
    );
    let inv = &invoices[0];
    assert_eq!(inv.invoice_type, InvoiceType::Recurring);
    assert_eq!(
        inv.status,
        InvoiceStatusEnum::Closed,
        "an empty renewal is Closed, not finalized as a EUR 0 invoice"
    );
    assert_eq!(inv.total, 0);
    assert_eq!(
        inv.invoice_number, "draft",
        "a closed invoice never consumes a sequential number (no gap)"
    );
}

/// Closing is not a one-shot: consecutive empty periods must each close.
///
/// Regression: closed invoices keep the placeholder `invoice_number = "draft"`, and the partial
/// unique index on `(invoice_number, invoicing_entity_id)` excluded only `DRAFT`. The first close
/// therefore entered the index as ('draft', entity) and the second collided, aborting the
/// settlement transaction — so every empty period after the first failed to settle, permanently.
#[tokio::test]
async fn test_consecutive_empty_periods_all_close() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    seed_usage_only_plan(&env).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Two renewals, both with zero usage, both under the same invoicing entity.
    env.process_cycles().await;
    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(
        invoices.len(),
        2,
        "each empty renewal period gets its own draft anchor"
    );
    assert!(
        invoices
            .iter()
            .all(|i| i.status == InvoiceStatusEnum::Closed),
        "every empty period closes, not just the first: {:?}",
        invoices.iter().map(|i| &i.status).collect::<Vec<_>>()
    );
    assert!(
        invoices.iter().all(|i| i.invoice_number == "draft"),
        "no closed invoice consumes a sequential number"
    );
}

/// The same usage-only renewal, but with usage imported for the period, must finalize normally.
/// This is the "usage arrived (in time or during grace)" side of the fix.
#[tokio::test]
async fn test_usage_only_renewal_with_usage_is_finalized() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let period2_end = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    // 100 units x EUR 0.10 = EUR 10.00 = 1000 cents for the first (arrears) period.
    let usage_client = build_usage_client(
        Decimal::from(100),
        &[(start_date, period1_end), (period1_end, period2_end)],
    );
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage_client)).await;
    seed_usage_only_plan(&env).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    assert_eq!(
        env.get_invoices(sub_id).await.len(),
        0,
        "no cycle-0 invoice for a purely-usage plan"
    );

    env.process_cycles().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 1);
    let inv = &invoices[0];
    assert_eq!(
        inv.status,
        InvoiceStatusEnum::Finalized,
        "usage arrived, so the renewal is billed and finalized"
    );
    assert_eq!(inv.total, 1000);
    assert_ne!(
        inv.invoice_number, "draft",
        "a finalized invoice has a real sequential number"
    );
}

/// The motivating scenario end-to-end: a usage-only renewal draft is created *empty* (no usage
/// imported by period close), then usage is imported *during the grace period* (after the draft
/// already exists), and the scheduled finalization picks it up — the invoice finalizes with the
/// correct amount instead of being Closed. This exercises the empty-draft -> refresh-at-finalize
/// path that a frozen mock cannot.
#[tokio::test]
async fn test_usage_only_late_import_during_grace_finalizes_with_usage() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    // Empty at first: no usage for any period.
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    // Positive grace so the renewal *schedules* finalization (rather than finalizing inline),
    // leaving a window in which the late usage import lands.
    env.set_consolidate_and_grace(false, 720).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Renewal creates the draft that bills period 0 — still empty, since no usage is imported yet.
    env.process_cycle_transitions().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(
        invoices.len(),
        1,
        "the empty renewal draft is created as the finalization anchor"
    );
    assert_eq!(
        invoices[0].status,
        InvoiceStatusEnum::Draft,
        "it is still an editable draft before finalization runs"
    );
    assert_eq!(invoices[0].total, 0);

    // Usage for period 0 is imported LATE, during the grace period, after the draft already exists.
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start_date.and_time(NaiveTime::MIN),
            period_end: period1_end.and_time(NaiveTime::MIN),
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod {
                start: start_date.and_time(NaiveTime::MIN),
                end: period1_end.and_time(NaiveTime::MIN),
            },
        },
    );

    // The scheduled finalization refreshes from current usage and bills the late import.
    env.process_due_events().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 1);
    let inv = &invoices[0];
    assert_eq!(
        inv.status,
        InvoiceStatusEnum::Finalized,
        "late-imported usage is captured at finalization, not lost"
    );
    assert_eq!(inv.total, 1000, "100 units x EUR 0.10 = 1000 cents");
    assert_ne!(inv.invoice_number, "draft");
}

/// A **manual** finalize (the console/API "Finalize" action) of an empty usage-only renewal draft
/// settles the period exactly like the automated grace-period finalize: it recomputes and bills
/// usage imported after the draft was created, instead of finalizing the stale stored EUR 0. The
/// two paths must not diverge — a manual finalize of a recurring period draft *is* settling that
/// period.
#[tokio::test]
async fn test_manual_finalize_bills_usage_imported_after_draft() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    // Positive grace so the renewal creates a draft and *schedules* finalization (rather than
    // finalizing inline), leaving a live Draft for us to finalize by hand.
    env.set_consolidate_and_grace(false, 720).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Renewal creates the empty draft anchor (no usage imported yet).
    env.process_cycle_transitions().await;
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].status, InvoiceStatusEnum::Draft);
    let invoice_id = invoices[0].id;

    // Usage for the settled period is imported *after* the draft exists, *before* the manual finalize.
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start_date.and_time(NaiveTime::MIN),
            period_end: period1_end.and_time(NaiveTime::MIN),
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod {
                start: start_date.and_time(NaiveTime::MIN),
                end: period1_end.and_time(NaiveTime::MIN),
            },
        },
    );

    let finalized = env
        .manual_finalize_invoice(invoice_id)
        .await
        .expect("manual finalize should succeed");

    assert_eq!(
        finalized.invoice.status,
        InvoiceStatusEnum::Finalized,
        "usage arrived, so the manual finalize bills it — not Close and not a stale EUR 0"
    );
    assert_eq!(
        finalized.invoice.total, 1000,
        "100 units × EUR 0.10 = EUR 10, billed on manual finalize just like the scheduler"
    );
    assert_ne!(finalized.invoice.invoice_number, "draft");
}

/// The other side of settle-on-manual-finalize: with genuinely no usage by finalization time, a
/// manual finalize of the empty recurring draft recomputes to nothing and `Close`s it — no
/// sequential number, no EUR 0 facture — identically to the automated settlement. Closing here is
/// safe precisely because it follows a *fresh recompute* (`refresh = true`), never stale stored
/// lines; that is the freshness contract the terminal-Close path requires.
#[tokio::test]
async fn test_manual_finalize_of_still_empty_recurring_draft_closes() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // No usage is ever imported → the period genuinely has nothing to bill.
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, 720).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.process_cycle_transitions().await;
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].status, InvoiceStatusEnum::Draft);
    let invoice_id = invoices[0].id;

    let finalized = env
        .manual_finalize_invoice(invoice_id)
        .await
        .expect("manual finalize should succeed");

    assert_eq!(
        finalized.invoice.status,
        InvoiceStatusEnum::Closed,
        "no usage by finalization → Close (fresh recompute), consistent with the scheduler; not a \
         stale EUR 0"
    );
    assert_eq!(finalized.invoice.total, 0);
    assert_eq!(
        finalized.invoice.invoice_number, "draft",
        "a Closed invoice consumes no sequential number"
    );
}

/// A usage-only subscription terminated **during its first period** (cycle 0, before any renewal)
/// must still create the empty draft anchor, so late usage for that final partial period can be
/// captured. Previously the anchor was gated on `cycle_index > 0`, so a first-period termination
/// skipped it entirely and any late usage had nothing to attach to.
#[tokio::test]
async fn test_first_period_termination_creates_draft_anchor() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let cancel_date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

    // No usage imported yet -> the termination invoice computes empty.
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    seed_usage_only_plan(&env).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Terminate mid first period, before any renewal — cycle_index is still 0.
    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            Some("first-period termination".to_string()),
            CancellationEffectiveAt::Date(cancel_date),
        )
        .await
        .expect("cancel_subscription failed");

    // Fire ONLY the scheduled cancel (a due event), not cycle transitions — otherwise the period-1
    // renewal would run first and bump cycle_index to 1, creating the anchor via the renewal path
    // and masking the cycle-0 termination gap this test targets.
    env.process_due_events().await;

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.status, SubscriptionStatusEnum::Cancelled);

    // The termination must anchor the final partial period (dated at the cancellation) even though
    // it computed empty and this is cycle 0. Without the fix that anchor is skipped entirely
    // (`is_renewal = cycle_index > 0` is false), so late usage for that period would be lost.
    let invoices = env.get_invoices(sub_id).await;
    assert!(
        invoices
            .iter()
            .any(|i| i.billing_period_start == cancel_date
                && i.invoice_type == InvoiceType::Recurring),
        "a first-period (cycle 0) termination must create the empty recurring anchor dated at the \
         termination; got {:?}",
        invoices
            .iter()
            .map(|i| (i.billing_period_start, i.invoice_type.clone(), i.status))
            .collect::<Vec<_>>()
    );
}

/// The anchor is only half the first-period-termination fix: the settle must also be able to BILL
/// the partial first period. `calculate_component_period_for_invoice_date` used to return
/// `arrear: None` for every cycle-0 invoice, so the anchor recomputed to nothing no matter how much
/// usage arrived — and Closed, permanently forfeiting the period. A period that settles early
/// (`is_completed`) bills arrears [billing_start, termination_date] even at cycle 0.
#[tokio::test]
async fn test_first_period_termination_bills_late_usage() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let cancel_date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    // Grace so the termination anchors a Draft and schedules the settle instead of closing inline.
    env.set_consolidate_and_grace(false, 720).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            Some("first-period termination with late usage".to_string()),
            CancellationEffectiveAt::Date(cancel_date),
        )
        .await
        .expect("cancel_subscription failed");

    // The scheduled cancel terminates the subscription and anchors the empty cycle-0 draft.
    env.process_due_events().await;
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].status, InvoiceStatusEnum::Draft);
    let anchor_id = invoices[0].id;

    // The final partial period's usage lands only now, during the grace window.
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start_date.and_time(NaiveTime::MIN),
            period_end: cancel_date.and_time(NaiveTime::MIN),
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod {
                start: start_date.and_time(NaiveTime::MIN),
                end: cancel_date.and_time(NaiveTime::MIN),
            },
        },
    );

    // The deferred settle recomputes the cycle-0 arrears window [start, cancel] and bills it.
    env.process_due_events().await;

    let invoices = env.get_invoices(sub_id).await;
    let inv = invoices.iter().find(|i| i.id == anchor_id).unwrap();
    assert_eq!(
        inv.status,
        InvoiceStatusEnum::Finalized,
        "late usage arrived: the anchor must bill it, not Close the period away"
    );
    assert_eq!(inv.total, 1000, "100 units x EUR 0.10 over [start, cancel]");

    // Churn was booked exactly once (at anchor insertion via process_mrr): MRR ends at zero.
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(
        sub.mrr_cents, 0,
        "cancelling a usage-only subscription must leave MRR at exactly 0"
    );
}

/// A usage-only subscription started and **immediately cancelled the same day** must still bill the
/// usage consumed that day. The date-granular arrears window is empty ([D, D)) and would drop it;
/// `CancellationEffectiveAt::Immediate` records the termination instant so the terminal period bills
/// [D 00:00, instant). Regression: an immediate cancel resolved to `Date(today)`, truncating the
/// usage window to midnight and silently forfeiting the day's usage the preview had shown as owed.
///
/// Runs on the real current date because the immediate path stamps `terminated_at` from the wall
/// clock; the mock returns usage for any non-empty window, so an empty [D, D) yields nothing (as a
/// real metering backend would) — the assertion holds only once the instant extends the window.
#[tokio::test]
async fn test_immediate_cancel_same_day_bills_usage() {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    usage.set_usage_for_any_nonempty_window(
        METRIC_BANDWIDTH,
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod {
                start: NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .and_time(NaiveTime::MIN),
                end: NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .and_time(NaiveTime::MIN),
            },
        },
    );
    // Negative grace settles the termination inline (recompute now), so the closing invoice is
    // finalized within this test instead of awaiting a scheduled grace finalization.
    env.set_consolidate_and_grace(false, -1).await;

    let today = chrono::Utc::now().date_naive();
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(today)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            Some("same-day immediate cancel".to_string()),
            CancellationEffectiveAt::Immediate,
        )
        .await
        .expect("cancel_subscription failed");

    // Fires the scheduled cancel (due at today's midnight); with negative grace the terminal period
    // settles inline in the same pass.
    env.process_due_events().await;

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.status, SubscriptionStatusEnum::Cancelled);
    assert!(
        sub.terminated_at.is_some(),
        "immediate cancel records the termination instant"
    );

    let invoices = env.get_invoices(sub_id).await;
    let inv = invoices
        .iter()
        .find(|i| i.invoice_type == InvoiceType::Recurring)
        .expect("a closing recurring invoice must exist");
    assert_eq!(
        inv.status,
        InvoiceStatusEnum::Finalized,
        "same-day usage is billed on the closing invoice, not Closed away"
    );
    assert_eq!(inv.total, 1000, "100 units x EUR 0.10 = 1000 cents");
    assert_ne!(
        inv.invoice_number, "draft",
        "a finalized closing invoice carries a real number"
    );
}

/// A usage-metered subscription cancelled **effective today by date** (not immediately) on the day
/// it started must settle cleanly. Its terminal arrear window is [today, today] with no instant
/// cutoff (date/pause cancels leave `terminated_at` null), so it stays empty and nothing is billed —
/// the usage query must short-circuit rather than send an empty window to the metering client, which
/// rejects it. Regression guard for the immediate-cancel fix, which keeps zero-width arrears: without
/// the short-circuit the closing settle errored on the empty window and the subscription could not
/// terminate. The mock rejects empty windows like production, so this fails loudly if it regresses.
#[tokio::test]
async fn test_date_cancel_same_day_settles_without_error() {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    // Usage is available for any non-empty window; a date cancel's window is empty, so it is never
    // reached — proving a same-day date cancel bills nothing (unlike an immediate cancel).
    usage.set_usage_for_any_nonempty_window(
        METRIC_BANDWIDTH,
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod {
                start: NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .and_time(NaiveTime::MIN),
                end: NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .and_time(NaiveTime::MIN),
            },
        },
    );
    env.set_consolidate_and_grace(false, -1).await;

    let today = chrono::Utc::now().date_naive();
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(today)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            Some("same-day date cancel".to_string()),
            CancellationEffectiveAt::Date(today),
        )
        .await
        .expect("cancel_subscription failed");

    // Must not error on the empty usage window.
    env.process_due_events().await;

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.status, SubscriptionStatusEnum::Cancelled);
    assert!(
        sub.terminated_at.is_none(),
        "a date-effective cancel records no instant cutoff"
    );
    let invoices = env.get_invoices(sub_id).await;
    let inv = invoices
        .iter()
        .find(|i| i.invoice_type == InvoiceType::Recurring)
        .expect("a closing recurring invoice must exist");
    assert_eq!(
        inv.status,
        InvoiceStatusEnum::Closed,
        "an empty same-day date cancel closes cleanly rather than erroring or billing"
    );
}

/// All-empty consolidation: two same-day usage-only renewals for one customer, both with no usage,
/// merge into a consolidated parent that has nothing to bill. The parent must be Closed (no EUR 0
/// facture, no sequential number) and the per-subscription children stay inert — linked to the
/// Closed parent, never finalized, contributing no revenue.
#[tokio::test]
async fn test_all_empty_consolidation_closes_parent() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // noop usage → every period is empty for both subscriptions.
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_recurring_invoices(true).await;

    // Same customer, same day, same currency/plan → the two renewals are eligible to merge.
    let sub_a = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let sub_b = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.process_cycles().await;

    // Exactly one consolidated parent (customer-level, no subscription_id), Closed with nothing to
    // bill rather than issued as a EUR 0 invoice.
    let parents: Vec<_> = env
        .get_customer_invoices(CUST_UBER_ID)
        .await
        .into_iter()
        .filter(|i| i.subscription_id.is_none() && i.invoice_type == InvoiceType::Recurring)
        .collect();
    assert_eq!(
        parents.len(),
        1,
        "expected a single consolidated parent, got {}",
        parents.len()
    );
    let parent = &parents[0];
    assert_eq!(
        parent.status,
        InvoiceStatusEnum::Closed,
        "an all-empty consolidation settles with nothing to bill → Closed, not a EUR 0 facture"
    );
    assert_eq!(parent.total, 0);
    assert_eq!(
        parent.invoice_number, "draft",
        "a closed parent consumes no sequential number (no gap)"
    );

    // Both per-subscription children are inert: linked to the Closed parent, zero revenue, and
    // never finalized on their own.
    let children = env.get_consolidated_children(parent.id).await;
    assert_eq!(children.len(), 2);
    for child in &children {
        assert_eq!(child.consolidated_into_invoice_id, Some(parent.id));
        assert_eq!(child.total, 0);
        assert_ne!(
            child.status,
            InvoiceStatusEnum::Finalized,
            "a consolidated child is never finalized on its own"
        );
    }
    let child_subs: Vec<_> = children.iter().filter_map(|c| c.subscription_id).collect();
    assert!(
        child_subs.contains(&sub_a) && child_subs.contains(&sub_b),
        "both subscriptions must appear as children of the consolidated parent"
    );
}

/// A draft that already carries billed lines must never `Close` because the recompute came back
/// empty — `Close` is terminal, so that forfeits the revenue for good. Here metering goes dark
/// mid-grace; the draft must stay open and retryable.
#[tokio::test]
async fn test_draft_with_lines_is_not_closed_when_recompute_comes_back_empty() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    // Positive grace: the renewal schedules finalization instead of settling inline, so the draft
    // exists with its lines before the settle runs.
    env.set_consolidate_and_grace(false, 720).await;

    let usage_params = || MockUsageDataParams {
        metric_id: METRIC_BANDWIDTH,
        period_start: start_date.and_time(NaiveTime::MIN),
        period_end: period1_end.and_time(NaiveTime::MIN),
    };

    // Usage is present when the renewal drafts period 0, so the draft is created with real lines.
    usage.set_usage(
        usage_params(),
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod {
                start: start_date.and_time(NaiveTime::MIN),
                end: period1_end.and_time(NaiveTime::MIN),
            },
        },
    );

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.process_cycle_transitions().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].status, InvoiceStatusEnum::Draft);
    assert_eq!(
        invoices[0].total, 1000,
        "draft carries 100 units x EUR 0.10 before the settle runs"
    );

    // Metering goes dark: the same period now reports no usage at all.
    usage.set_usage(
        usage_params(),
        UsageData {
            data: vec![],
            period: UsagePeriod {
                start: start_date.and_time(NaiveTime::MIN),
                end: period1_end.and_time(NaiveTime::MIN),
            },
        },
    );

    // The scheduled settle recomputes to nothing. It must refuse rather than Close.
    env.process_due_events().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 1);
    assert_ne!(
        invoices[0].status,
        InvoiceStatusEnum::Closed,
        "closing here would forfeit the period's billed usage with no way back"
    );
    assert_eq!(
        invoices[0].status,
        InvoiceStatusEnum::Draft,
        "the draft stays open so the settle can be retried once metering recovers"
    );
    assert_eq!(
        invoices[0].total, 1000,
        "the already-billed lines survive the failed settle"
    );
}

// A new version of the seeded Beta plan: monthly + quarterly, so cycle gating is observable.
const PLAN_VERSION_MIXED_CADENCE_ID: PlanVersionId =
    PlanVersionId::from_const(uuid::uuid!("019438e0-0d01-7000-8000-000000000001"));
const COMP_MIXED_MONTHLY_ID: PriceComponentId =
    PriceComponentId::from_const(uuid::uuid!("019438e0-0d02-7000-8000-000000000001"));
const COMP_MIXED_QUARTERLY_ID: PriceComponentId =
    PriceComponentId::from_const(uuid::uuid!("019438e0-0d03-7000-8000-000000000001"));
const PRICE_MIXED_MONTHLY_ID: PriceId =
    PriceId::from_const(uuid::uuid!("019438e0-0d04-7000-8000-000000000001"));
const PRICE_MIXED_QUARTERLY_ID: PriceId =
    PriceId::from_const(uuid::uuid!("019438e0-0d05-7000-8000-000000000001"));

/// A quarterly component alongside a monthly one, so the subscription itself stays monthly.
async fn seed_mixed_cadence_usage_plan(env: &TestEnv) {
    let mut conn = env.conn().await;
    PlanSeed::seed_version(
        &mut conn,
        PLAN_USAGE_BETA_ID,
        PLAN_VERSION_MIXED_CADENCE_ID,
        1,
        "EUR",
        &[
            SeedComp::usage(
                COMP_MIXED_MONTHLY_ID,
                "API Calls",
                PRODUCT_API_CALLS_ID,
                METRIC_BANDWIDTH,
                PRICE_MIXED_MONTHLY_ID,
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit {
                    rate: Decimal::new(10, 2), // EUR 0.10 / unit
                },
            ),
            SeedComp::usage(
                COMP_MIXED_QUARTERLY_ID,
                "DB Storage",
                PRODUCT_DB_STORAGE_ID,
                METRIC_DATABASE_SIZE,
                PRICE_MIXED_QUARTERLY_ID,
                DieselBillingPeriodEnum::Quarterly,
                UsagePricingModel::PerUnit {
                    rate: Decimal::new(50, 2), // EUR 0.50 / unit
                },
            ),
        ],
    )
    .await
    .expect("failed to seed mixed-cadence plan version");
}

/// A refresh gates components on the cycle the *invoice* belongs to, not the one the subscription
/// has since reached. A quarterly component bills every third cycle of a monthly subscription, so a
/// draft that outlives a renewal would otherwise be billed against a cycle that is not its own.
#[tokio::test]
async fn test_refresh_gates_components_on_the_invoice_cycle_not_the_current_one() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let mut data = std::collections::HashMap::new();
    for (metric, units) in [
        (METRIC_BANDWIDTH, Decimal::from(100)),
        (METRIC_DATABASE_SIZE, Decimal::from(40)),
    ] {
        data.insert(
            MockUsageDataParams {
                metric_id: metric,
                period_start: start_date.and_time(NaiveTime::MIN),
                period_end: period1_end.and_time(NaiveTime::MIN),
            },
            UsageData {
                data: vec![GroupedUsageData {
                    value: units,
                    dimensions: std::collections::HashMap::new(),
                }],
                period: UsagePeriod {
                    start: start_date.and_time(NaiveTime::MIN),
                    end: period1_end.and_time(NaiveTime::MIN),
                },
            },
        );
    }
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient { data })).await;
    seed_mixed_cadence_usage_plan(&env).await;
    // Grace so the renewal schedules finalization rather than settling inline.
    env.set_consolidate_and_grace(false, 720).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_MIXED_CADENCE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.process_cycle_transitions().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 1);
    let drafted_total = invoices[0].total;
    assert_eq!(
        drafted_total, 1000,
        "this period owes the monthly component only (100 x EUR 0.10); the quarterly one is not          due at this cycle"
    );

    // Renewals land the subscription on a cycle where the quarterly component *is* due; it belongs
    // to another period, so recomputing against the current cycle would bill it in here.
    env.advance_subscription_cycle(sub_id, 3).await;

    env.process_due_events().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 1);
    let inv = &invoices[0];
    assert_eq!(inv.status, InvoiceStatusEnum::Finalized);
    assert_eq!(
        inv.total, drafted_total,
        "the settle bills the invoice's own cycle, so no component from another period is pulled in"
    );
}

/// The advance/arrears split through the settle-refresh: a renewal draft bills the rate for the
/// period *ahead* (advance) and the usage of the period just *elapsed* (arrears). When the usage is
/// imported only during the grace window, the settle must add the arrears line for the elapsed
/// period while carrying the drafted advance line through untouched — each line on its own period.
#[tokio::test]
async fn test_settle_bills_advance_for_new_period_and_late_usage_for_elapsed_one() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
    let period2_end = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    env.set_consolidate_and_grace(false, 720).await;

    // Rate EUR 20/mo (advance) + Bandwidth EUR 0.10/unit (arrears).
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Renewal: the draft carries February's rate in advance; no usage line yet (nothing imported).
    env.process_cycle_transitions().await;
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 2);
    assert_eq!(invoices[1].status, InvoiceStatusEnum::Draft);
    assert_eq!(
        invoices[1].total, 2000,
        "the renewal draft bills only the advance rate before any usage is imported"
    );
    let renewal_id = invoices[1].id;

    // January's usage lands during the grace window, after the draft exists.
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start_date.and_time(NaiveTime::MIN),
            period_end: period1_end.and_time(NaiveTime::MIN),
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod {
                start: start_date.and_time(NaiveTime::MIN),
                end: period1_end.and_time(NaiveTime::MIN),
            },
        },
    );

    env.process_due_events().await;

    let invoices = env.get_invoices(sub_id).await;
    let inv = invoices.iter().find(|i| i.id == renewal_id).unwrap();
    assert_eq!(inv.status, InvoiceStatusEnum::Finalized);
    assert_eq!(
        inv.total, 3000,
        "EUR 20 advance rate + 100 units x EUR 0.10 late-imported arrears usage"
    );

    let rate_line = inv
        .line_items
        .iter()
        .find(|l| l.metric_id.is_none())
        .expect("the drafted advance rate line survives the settle");
    assert_eq!(
        (rate_line.start_date, rate_line.end_date),
        (period1_end, period2_end),
        "the advance line covers the period ahead of the invoice date"
    );

    let usage_line = inv
        .line_items
        .iter()
        .find(|l| l.metric_id == Some(METRIC_BANDWIDTH))
        .expect("the late-imported usage must be billed as an arrears line");
    assert_eq!(
        (usage_line.start_date, usage_line.end_date),
        (start_date, period1_end),
        "the arrears line covers the elapsed period, not the invoice's advance period"
    );
}

/// Cycle pinning applies to add-ons exactly as to price components: a quarterly usage add-on is due
/// on the invoice drafted at cycle 3, and must still be billed there when the subscription has
/// drifted past that cycle by settle time. Without the pin, the recompute gates the add-on against
/// the *current* cycle, silently dropping its line while the advance rate survives — an
/// under-billed invoice with no error anywhere.
#[tokio::test]
async fn test_refresh_gates_add_ons_on_the_invoice_cycle_not_the_current_one() {
    let jan1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let apr1 = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    env.set_consolidate_and_grace(false, 720).await;

    // The quarterly add-on bills [Jan 1, Apr 1] in arrears on the cycle-3 invoice:
    // 40 units x EUR 0.50 = EUR 20. The plan's own monthly metric never reports usage.
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_DATABASE_SIZE,
            period_start: jan1.and_time(NaiveTime::MIN),
            period_end: apr1.and_time(NaiveTime::MIN),
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(40),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod {
                start: jan1.and_time(NaiveTime::MIN),
                end: apr1.and_time(NaiveTime::MIN),
            },
        },
    );

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let add_on_id = env
        .store()
        .create_add_on_from_ref(
            "DB Storage".to_string(),
            ProductRef::New {
                name: "DB Storage Product".to_string(),
                fee_type: FeeTypeEnum::Usage,
                fee_structure: FeeStructure::Usage {
                    metric_id: METRIC_DATABASE_SIZE,
                    model: UsageModel::PerUnit,
                },
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Quarterly,
                currency: "EUR".to_string(),
                pricing: Pricing::Usage(UsagePricingModel::PerUnit {
                    rate: Decimal::new(50, 2),
                }),
                credit_unit: None,
            }),
            None,
            true,
            None,
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("create usage add-on failed")
        .id;

    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![CreateSubscriptionAddOn {
                        add_on_id,
                        customization: SubscriptionAddOnCustomization::None,
                        quantity: 1,
                    }],
                    edited: vec![],
                    removed: vec![],
                },
            },
            jan1,
        )
        .await
        .expect("add usage add-on failed");

    // Three renewals: Feb (cycle 1) and Mar (cycle 2) draft the rate only; Apr (cycle 3) also owes
    // the quarterly add-on's arrears segment.
    env.process_cycle_transitions().await;
    env.process_cycle_transitions().await;
    env.process_cycle_transitions().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 4, "creation invoice + three renewal drafts");
    let apr_draft = &invoices[3];
    assert_eq!(apr_draft.status, InvoiceStatusEnum::Draft);
    assert_eq!(
        apr_draft.total, 4000,
        "EUR 20 advance rate + EUR 20 quarterly add-on usage drafted at cycle 3"
    );
    let apr_id = apr_draft.id;

    // The subscription drifts past the drafted cycle before the settle runs; 5 % 3 != 0, so the
    // current cycle would gate the quarterly add-on out of its own invoice.
    env.advance_subscription_cycle(sub_id, 5).await;
    env.process_due_events().await;

    let invoices = env.get_invoices(sub_id).await;
    let apr = invoices.iter().find(|i| i.id == apr_id).unwrap();
    assert_eq!(apr.status, InvoiceStatusEnum::Finalized);
    assert_eq!(
        apr.total, 4000,
        "the add-on line is gated on the invoice's own cycle: dropping it would under-bill"
    );
    let addon_line = apr
        .line_items
        .iter()
        .find(|l| l.metric_id == Some(METRIC_DATABASE_SIZE))
        .expect("the quarterly add-on usage line must survive the settle");
    assert_eq!(
        (addon_line.start_date, addon_line.end_date),
        (jan1, apr1),
        "the add-on bills its full quarterly arrears window"
    );
}

/// Slots ride the same settle path as everything else: an advance slot line is not usage-based, so
/// the settle reuses the drafted line as-is — cycle drift must neither re-derive it against the
/// wrong period nor Close the draft. The scheduled settle that fires after a manual finalize must
/// treat the already-finalized invoice as an idempotent no-op, not an error and not a re-bill.
#[tokio::test]
async fn test_slot_draft_settles_from_stored_lines_under_cycle_drift() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    env.set_consolidate_and_grace(false, 720).await;

    // Starter: Platform Fee EUR 29/mo (rate) + Seats EUR 10/seat/mo (slot, default 1 seat).
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.process_cycle_transitions().await;
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 2);
    let draft = &invoices[1];
    assert_eq!(draft.status, InvoiceStatusEnum::Draft);
    assert_eq!(draft.total, 3900, "EUR 29 rate + 1 seat x EUR 10 drafted");
    let draft_id = draft.id;

    // The subscription moves on before the operator clicks Finalize.
    env.advance_subscription_cycle(sub_id, 3).await;

    let finalized = env
        .manual_finalize_invoice(draft_id)
        .await
        .expect("manual finalize of a slot draft should succeed");
    assert_eq!(finalized.invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(
        finalized.invoice.total, 3900,
        "the drafted slot line settles as stored — drift neither re-bills nor drops it"
    );
    let seats_line = finalized
        .invoice
        .line_items
        .iter()
        .find(|l| l.name == "Seats")
        .expect("the slot line survives the settle");
    assert_eq!(seats_line.quantity, Some(Decimal::from(1)));

    // The renewal's scheduled settle now fires on an already-finalized invoice: idempotent skip.
    env.process_due_events().await;
    let invoices = env.get_invoices(sub_id).await;
    let inv = invoices.iter().find(|i| i.id == draft_id).unwrap();
    assert_eq!(inv.status, InvoiceStatusEnum::Finalized);
    assert_eq!(inv.total, 3900, "the scheduled settle must not re-bill");
}

/// The under-billing direction of cycle drift: the drafted invoice's *only* line belongs to a
/// component due at its own cycle (quarterly, drafted at cycle 3). By settle time the subscription
/// sits at cycle 5, where that component is gated out — an unpinned recompute comes back empty and
/// the settle would Close the period outright, forfeiting its revenue with no error anywhere. The
/// pin keeps the recompute on the invoice's own cycle so the period bills what it owed.
#[tokio::test]
async fn test_component_due_this_period_still_bills_when_the_cycle_drifted() {
    let jan1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let apr1 = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_mixed_cadence_usage_plan(&env).await;
    env.set_consolidate_and_grace(false, 720).await;

    // Only the quarterly metric ever reports: 40 units x EUR 0.50 over [Jan 1, Apr 1] = EUR 20.
    // The monthly metric stays silent, so the quarterly line is the Apr draft's only line.
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_DATABASE_SIZE,
            period_start: jan1.and_time(NaiveTime::MIN),
            period_end: apr1.and_time(NaiveTime::MIN),
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(40),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod {
                start: jan1.and_time(NaiveTime::MIN),
                end: apr1.and_time(NaiveTime::MIN),
            },
        },
    );

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_MIXED_CADENCE_ID)
        .start_date(jan1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Feb and Mar renewals draft empty anchors (monthly usage 0, quarterly not due); the Apr
    // renewal (cycle 3) drafts the quarterly component's line.
    env.process_cycle_transitions().await;
    env.process_cycle_transitions().await;
    env.process_cycle_transitions().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 3);
    let apr_draft = &invoices[2];
    assert_eq!(apr_draft.status, InvoiceStatusEnum::Draft);
    assert_eq!(
        apr_draft.total, 2000,
        "the cycle-3 draft owes exactly the quarterly component's usage"
    );
    let apr_id = apr_draft.id;

    // Drift: at cycle 5 the quarterly component is gated out (5 % 3 != 0), so an unpinned
    // recompute of the Apr draft comes back empty.
    env.advance_subscription_cycle(sub_id, 5).await;
    env.process_due_events().await;

    let invoices = env.get_invoices(sub_id).await;
    let apr = invoices.iter().find(|i| i.id == apr_id).unwrap();
    assert_eq!(
        apr.status,
        InvoiceStatusEnum::Finalized,
        "the period had revenue: it must bill, not Close (forfeit) and not stall as Draft"
    );
    assert_eq!(apr.total, 2000, "the settle bills what the period owed");
    assert_ne!(apr.invoice_number, "draft");

    // The two genuinely empty periods Close as usual — drift does not disturb them.
    for inv in invoices.iter().filter(|i| i.id != apr_id) {
        assert_eq!(
            inv.status,
            InvoiceStatusEnum::Closed,
            "an empty elapsed period settles Closed"
        );
    }
}

/// A manual finalize of a draft whose billing content an operator edited must issue exactly what
/// they previewed. Routing it through the settle-refresh would regenerate the usage lines and
/// derive the total from coupons alone — silently dropping the manual discount from the total
/// while the discount column keeps it, an invoice that doesn't reconcile and overcharges.
#[tokio::test]
async fn test_manual_finalize_of_hand_edited_draft_keeps_the_edit() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    env.set_consolidate_and_grace(false, 720).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    // Renewal draft: February's rate (EUR 20), no usage line yet.
    env.process_cycle_transitions().await;
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 2);
    assert_eq!(invoices[1].total, 2000);
    let draft_id = invoices[1].id;

    // The operator grants a EUR 5 invoice discount on the draft.
    let edited = env
        .services()
        .update_draft_invoice(
            draft_id,
            TENANT_ID,
            meteroid_store::domain::UpdateInvoiceParams {
                memo: None,
                reference: None,
                purchase_order: None,
                due_date: None,
                line_items: None,
                discount: Some("5.00".to_string()),
                customer_details: None,
                invoicing_entity_id: None,
                annexes: None,
                custom_properties: None,
            },
        )
        .await
        .expect("draft edit should succeed");
    assert_eq!(edited.invoice.total, 1500, "EUR 20 - EUR 5 discount");
    assert_eq!(edited.invoice.discount, 500);

    // Usage lands after the edit; a blind settle-refresh would pull it in and drop the discount.
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start_date.and_time(NaiveTime::MIN),
            period_end: period1_end.and_time(NaiveTime::MIN),
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod {
                start: start_date.and_time(NaiveTime::MIN),
                end: period1_end.and_time(NaiveTime::MIN),
            },
        },
    );

    let finalized = env
        .manual_finalize_invoice(draft_id)
        .await
        .expect("manual finalize of an edited draft should succeed");
    assert_eq!(finalized.invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(
        finalized.invoice.total, 1500,
        "the operator finalized what they previewed: the edited total stands"
    );
    assert_eq!(
        finalized.invoice.discount, 500,
        "discount column and total agree — the invoice reconciles"
    );
    assert_ne!(finalized.invoice.invoice_number, "draft");
}

/// The scheduler must honor operator edits exactly like the manual path: when the grace window
/// expires on a hand-edited draft, the scheduled settle finalizes what the operator previewed —
/// no refresh pulling late usage in over the edit, no dropped discount, no Close, no merge.
#[tokio::test]
async fn test_scheduled_settle_of_hand_edited_draft_keeps_the_edit() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    env.set_consolidate_and_grace(false, 720).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.process_cycle_transitions().await;
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 2);
    let draft_id = invoices[1].id;

    let edited = env
        .services()
        .update_draft_invoice(
            draft_id,
            TENANT_ID,
            meteroid_store::domain::UpdateInvoiceParams {
                memo: None,
                reference: None,
                purchase_order: None,
                due_date: None,
                line_items: None,
                discount: Some("5.00".to_string()),
                customer_details: None,
                invoicing_entity_id: None,
                annexes: None,
                custom_properties: None,
            },
        )
        .await
        .expect("draft edit should succeed");
    assert_eq!(edited.invoice.total, 1500);

    // Usage lands before the grace expiry; a refresh would bill it over the operator's edit.
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start_date.and_time(NaiveTime::MIN),
            period_end: period1_end.and_time(NaiveTime::MIN),
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod {
                start: start_date.and_time(NaiveTime::MIN),
                end: period1_end.and_time(NaiveTime::MIN),
            },
        },
    );

    // The scheduled settle fires at grace expiry.
    env.process_due_events().await;

    let invoices = env.get_invoices(sub_id).await;
    let inv = invoices.iter().find(|i| i.id == draft_id).unwrap();
    assert_eq!(inv.status, InvoiceStatusEnum::Finalized);
    assert_eq!(
        inv.total, 1500,
        "the scheduler issues the operator's edited total, same as the manual path"
    );
    assert_eq!(inv.discount, 500, "discount column and total agree");
    assert_ne!(inv.invoice_number, "draft");
}

/// A failed settle must leave the scheduled event retryable and commit NOTHING else: the runner
/// runs the handler in a savepoint, so the retry marker is the only write that survives the
/// failure. Metering going dark makes the settle refuse (see the no-Close guard); the event must
/// come back Pending with a bumped retry count — not Completed, not Failed, not half-applied.
#[tokio::test]
async fn test_failed_settle_leaves_the_event_retryable() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, 720).await;

    let usage_params = || MockUsageDataParams {
        metric_id: METRIC_BANDWIDTH,
        period_start: start_date.and_time(NaiveTime::MIN),
        period_end: period1_end.and_time(NaiveTime::MIN),
    };
    usage.set_usage(
        usage_params(),
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: HashMap::new(),
            }],
            period: UsagePeriod {
                start: start_date.and_time(NaiveTime::MIN),
                end: period1_end.and_time(NaiveTime::MIN),
            },
        },
    );

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.process_cycle_transitions().await;
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].total, 1000);

    // Metering goes dark; the deferred settle refuses to wipe the billed lines.
    usage.set_usage(
        usage_params(),
        UsageData {
            data: vec![],
            period: UsagePeriod {
                start: start_date.and_time(NaiveTime::MIN),
                end: period1_end.and_time(NaiveTime::MIN),
            },
        },
    );
    env.process_due_events().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices[0].status, InvoiceStatusEnum::Draft);
    assert_eq!(invoices[0].total, 1000, "billed lines survive the failure");

    let mut conn = env.conn().await;
    let pending =
        diesel_models::scheduled_events::ScheduledEventRow::get_pending_events_for_subscription(
            &mut conn, sub_id, &TENANT_ID,
        )
        .await
        .expect("listing pending events should succeed");
    let finalize_event = pending
        .iter()
        .find(|e| e.event_type == diesel_models::enums::ScheduledEventTypeEnum::FinalizeInvoice)
        .expect("the failed settle must leave its event Pending (retryable), not Completed/Failed");
    assert_eq!(
        finalize_event.retries, 1,
        "exactly one failed attempt is recorded on the retry marker"
    );
}

/// The refuse-to-wipe rule must also hold under consolidation: `build_and_finalize_consolidated`
/// refreshes every member before merging, and a member whose billed lines recompute to nothing
/// (metering outage) must abort the settle — not be zeroed on disk and merged into a parent that
/// under-bills the customer.
#[tokio::test]
async fn test_consolidation_refuses_when_a_member_recomputes_empty() {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let period1_end = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(true, 720).await;

    let usage_params = || MockUsageDataParams {
        metric_id: METRIC_BANDWIDTH,
        period_start: start_date.and_time(NaiveTime::MIN),
        period_end: period1_end.and_time(NaiveTime::MIN),
    };
    let usage_data = |units: u32| UsageData {
        data: vec![GroupedUsageData {
            value: Decimal::from(units),
            dimensions: HashMap::new(),
        }],
        period: UsagePeriod {
            start: start_date.and_time(NaiveTime::MIN),
            end: period1_end.and_time(NaiveTime::MIN),
        },
    };
    // Usage present while the renewals draft, so both members carry billed lines.
    usage.set_usage(usage_params(), usage_data(100));

    let sub_a = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let sub_b = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.process_cycle_transitions().await;
    for sub in [sub_a, sub_b] {
        let invoices = env.get_invoices(sub).await;
        assert_eq!(invoices.len(), 1);
        assert_eq!(invoices[0].total, 1000, "drafted with 100 x EUR 0.10");
    }

    // Metering goes dark before the deferred settle runs.
    usage.set_usage(
        usage_params(),
        UsageData {
            data: vec![],
            period: UsagePeriod {
                start: start_date.and_time(NaiveTime::MIN),
                end: period1_end.and_time(NaiveTime::MIN),
            },
        },
    );

    env.process_due_events().await;

    // The consolidation refused: no member was zeroed, nothing was merged, no parent exists.
    for sub in [sub_a, sub_b] {
        let invoices = env.get_invoices(sub).await;
        assert_eq!(invoices.len(), 1);
        assert_eq!(invoices[0].status, InvoiceStatusEnum::Draft);
        assert_eq!(
            invoices[0].total, 1000,
            "the billed lines survive the refused settle and stay retryable"
        );
        assert_eq!(invoices[0].consolidated_into_invoice_id, None);
    }
    let parents: Vec<_> = env
        .get_customer_invoices(CUST_UBER_ID)
        .await
        .into_iter()
        .filter(|i| i.subscription_id.is_none() && i.invoice_type == InvoiceType::Recurring)
        .collect();
    assert!(
        parents.is_empty(),
        "no consolidated parent may be built from a wiped member"
    );
}

const PLAN_VERSION_USAGE_SETUP_ID: PlanVersionId =
    PlanVersionId::from_const(uuid!("019438e0-0f04-7000-8000-0000000000f4"));
const COMP_USAGE_SETUP_API_CALLS_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0f05-7000-8000-0000000000f5"));
const PRICE_USAGE_SETUP_API_CALLS_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0f06-7000-8000-0000000000f6"));
const COMP_USAGE_SETUP_FEE_ID: PriceComponentId =
    PriceComponentId::from_const(uuid!("019438e0-0f07-7000-8000-0000000000f7"));
const PRICE_USAGE_SETUP_FEE_ID: PriceId =
    PriceId::from_const(uuid!("019438e0-0f08-7000-8000-0000000000f8"));

async fn seed_usage_with_setup_fee_plan(env: &TestEnv) {
    let mut conn = env.conn().await;
    PlanSeed::seed_version(
        &mut conn,
        PLAN_USAGE_ALPHA_ID,
        PLAN_VERSION_USAGE_SETUP_ID,
        91,
        "EUR",
        &[
            SeedComp::usage(
                COMP_USAGE_SETUP_API_CALLS_ID,
                "API Calls",
                PRODUCT_API_CALLS_ID,
                METRIC_BANDWIDTH,
                PRICE_USAGE_SETUP_API_CALLS_ID,
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit {
                    rate: Decimal::new(10, 2),
                },
            ),
            SeedComp::one_time(
                COMP_USAGE_SETUP_FEE_ID,
                "Setup Fee",
                PRODUCT_ONETIME_SETUP_ID,
                PRICE_USAGE_SETUP_FEE_ID,
                Decimal::new(20000, 2),
                1,
            ),
        ],
    )
    .await
    .expect("failed to seed usage + setup fee plan version");
}

async fn cancel_in_first_period_with_setup_fee(grace_hours: i32) -> Vec<Invoice> {
    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let cancel_date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

    let usage_client = build_usage_client(Decimal::from(100), &[(start_date, cancel_date)]);
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage_client)).await;
    seed_usage_with_setup_fee_plan(&env).await;
    env.set_consolidate_and_grace(false, grace_hours).await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_SETUP_ID)
        .start_date(start_date)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            Some("first-period termination with a setup fee".to_string()),
            CancellationEffectiveAt::Date(cancel_date),
        )
        .await
        .expect("cancel_subscription failed");

    env.process_due_events().await;
    env.process_due_events().await;

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.status, SubscriptionStatusEnum::Cancelled);
    env.get_invoices(sub_id).await
}

/// A plan-level one-time fee bills on the first invoice only. A subscription terminated in its first
/// period closes on a cycle-0 recurring invoice, which must bill that period's usage but not the
/// one-time fee a second time.
#[tokio::test]
async fn test_first_period_termination_does_not_rebill_one_time_fee() {
    let invoices = cancel_in_first_period_with_setup_fee(-1).await;
    invoices.assert().has_count(2);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("first invoice: the setup fee")
        .has_total(20000);
    invoices
        .assert()
        .invoice_at(1)
        .with_context("closing invoice: 100 units x EUR 0.10, no second setup fee")
        .has_total(1000);
}

/// The first invoice is still a draft in its grace period when the subscription terminates: its
/// recompute at finalization must keep the setup fee it was created with.
#[tokio::test]
async fn test_first_period_termination_keeps_one_time_fee_on_draft_first_invoice() {
    let invoices = cancel_in_first_period_with_setup_fee(720).await;
    let summary: Vec<_> = invoices
        .iter()
        .map(|i| {
            (
                i.billing_period_start,
                i.invoice_type.clone(),
                i.status,
                i.total,
            )
        })
        .collect();
    assert_eq!(invoices.len(), 2, "{summary:?}");
    assert_eq!(
        invoices[0].total, 20000,
        "first invoice keeps the setup fee: {summary:?}"
    );
    assert_eq!(
        invoices[1].total, 1000,
        "closing invoice bills usage only: {summary:?}"
    );
}

/// Same-day immediate cancel: the closing invoice shares the first invoice's date, and must still
/// leave the setup fee to the first invoice.
#[tokio::test]
async fn test_same_day_cancel_does_not_rebill_one_time_fee() {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_with_setup_fee_plan(&env).await;
    env.set_consolidate_and_grace(false, -1).await;

    let today = chrono::Utc::now().date_naive();
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_USAGE_SETUP_ID)
        .start_date(today)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            Some("same-day immediate cancel with a setup fee".to_string()),
            CancellationEffectiveAt::Immediate,
        )
        .await
        .expect("cancel_subscription failed");
    env.process_due_events().await;

    let invoices = env.get_invoices(sub_id).await;
    let summary: Vec<_> = invoices
        .iter()
        .map(|i| {
            (
                i.billing_period_start,
                i.invoice_type.clone(),
                i.status,
                i.total,
            )
        })
        .collect();
    let billed: i64 = invoices
        .iter()
        .filter(|i| i.status != InvoiceStatusEnum::Void)
        .map(|i| i.total)
        .sum();
    assert_eq!(
        billed, 20000,
        "the setup fee bills exactly once: {summary:?}"
    );
}
