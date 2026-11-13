//! A plan switch at a period's end that changes the cadence (monthly to quarterly, quarterly to
//! monthly, monthly to annual): periods after it run on the new cadence from the switch, each fee
//! is billed once per period and every period's usage is billed once.
use std::sync::Arc;

use chrono::NaiveDate;
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId, ProductId, SubscriptionId,
};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::price_components::UsagePricingModel;
use meteroid_store::domain::{BillingType, Invoice, LineItemKind, UsagePeriod};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

async fn usage_env() -> (TestEnv, MutableMockUsageClient) {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    (env, usage)
}

/// Bandwidth units used over exactly `[from, to)`: a window the engine asks for any other way
/// reads nothing, so a misplaced arrears window shows as a missing usage line.
fn set_usage(usage: &MutableMockUsageClient, from: NaiveDate, to: NaiveDate, units: i64) {
    let (start, end) = (
        from.and_time(chrono::NaiveTime::MIN),
        to.and_time(chrono::NaiveTime::MIN),
    );
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start,
            period_end: end,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(units),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod { start, end },
        },
    );
}

/// A Platform Fee of `fee` per period in advance and bandwidth at 0.10 a unit in arrears, both on
/// the plan's cadence.
async fn plan(
    env: &TestEnv,
    name: &'static str,
    period: DieselBillingPeriodEnum,
    fee: Decimal,
) -> PlanVersionId {
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), name, version_id)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                period.clone(),
                fee,
            ),
            SeedComp::usage(
                PriceComponentId::new(),
                "Bandwidth",
                PRODUCT_BANDWIDTH_ID,
                METRIC_BANDWIDTH,
                PriceId::new(),
                period,
                UsagePricingModel::PerUnit { rate: dec!(0.10) },
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    version_id
}

async fn start(env: &TestEnv, plan_version_id: PlanVersionId, on: NaiveDate) -> SubscriptionId {
    subscription()
        .plan_version(plan_version_id)
        .start_date(on)
        .on_start()
        .no_trial()
        .create(env.services())
        .await
}

async fn switch_at_period_end(env: &TestEnv, sub_id: SubscriptionId, to: PlanVersionId) {
    env.services()
        .schedule_plan_change(Actor::System, sub_id, TENANT_ID, to, vec![])
        .await
        .expect("a change at the period end");
}

async fn period(env: &TestEnv, sub_id: SubscriptionId) -> (NaiveDate, Option<NaiveDate>) {
    let sub = env.get_subscription(sub_id).await;
    (sub.current_period_start, sub.current_period_end)
}

/// As a release before this fix left a subscription switched to another cadence: its period ends
/// on the old cadence's grid. Fabricated state.
async fn leave_period_end(env: &TestEnv, sub_id: SubscriptionId, end: NaiveDate) {
    use diesel_models::schema::subscription::dsl as s;
    diesel::update(s::subscription.filter(s::id.eq(sub_id)))
        .set(s::current_period_end.eq(Some(end)))
        .execute(&mut env.conn().await)
        .await
        .expect("period end left off grid");
}

/// Every billed line of an invoice: what it is, its amount and its service period.
fn billed(invoice: &Invoice) -> Vec<(&'static str, i64, NaiveDate, NaiveDate)> {
    let mut lines: Vec<_> = invoice
        .line_items
        .iter()
        .filter(|l| l.amount_subtotal != 0)
        .map(|l| {
            (
                if l.metric_id.is_some() {
                    "usage"
                } else {
                    "fee"
                },
                l.amount_subtotal,
                l.start_date,
                l.end_date,
            )
        })
        .collect();
    lines.sort();
    lines
}

async fn invoice_of(env: &TestEnv, sub_id: SubscriptionId, on: NaiveDate) -> Invoice {
    let invoices = env.get_invoices(sub_id).await;
    invoices
        .iter()
        .find(|invoice| invoice.billing_period_start == on)
        .cloned()
        .unwrap_or_else(|| {
            let seen: Vec<_> = invoices
                .iter()
                .map(|i| (i.billing_period_start, billed(i)))
                .collect();
            panic!("no invoice starting {on}: {seen:?}")
        })
}

/// The renewals that billed something, by period start.
async fn billing_dates(env: &TestEnv, sub_id: SubscriptionId) -> Vec<NaiveDate> {
    let mut dates: Vec<NaiveDate> = env
        .get_invoices(sub_id)
        .await
        .iter()
        .filter(|invoice| !billed(invoice).is_empty())
        .map(|invoice| invoice.billing_period_start)
        .collect();
    dates.sort();
    dates
}

/// The fee's service periods over every invoice, in order: they must tile time, never overlap.
async fn fee_periods(env: &TestEnv, sub_id: SubscriptionId) -> Vec<(i64, NaiveDate, NaiveDate)> {
    let mut periods: Vec<_> = env
        .get_invoices(sub_id)
        .await
        .iter()
        .flat_map(billed)
        .filter(|(kind, ..)| *kind == "fee")
        .map(|(_, amount, from, to)| (amount, from, to))
        .collect();
    periods.sort_by_key(|(_, from, _)| *from);
    periods
}

/// Monthly 29.00 to quarterly 300.00 at Feb 1: Feb 1 settles January's usage and bills the
/// quarter [Feb 1, May 1); the next renewals are May 1 and Aug 1, each settling the quarter's
/// usage at 0.10 a unit and billing the next quarter.
#[tokio::test]
async fn a_monthly_to_quarterly_switch_renews_quarterly_from_the_switch() {
    let (env, usage) = usage_env().await;
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let quarterly = plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(300),
    )
    .await;
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 1, 1), day(2024, 2, 1), 1_000);
    set_usage(&usage, day(2024, 2, 1), day(2024, 5, 1), 3_000);
    set_usage(&usage, day(2024, 5, 1), day(2024, 8, 1), 2_000);
    switch_at_period_end(&env, sub_id, quarterly).await;

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 2, 1)).await),
        vec![
            ("fee", 30_000, day(2024, 2, 1), day(2024, 5, 1)),
            ("usage", 10_000, day(2024, 1, 1), day(2024, 2, 1)),
        ]
    );
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 2, 1), Some(day(2024, 5, 1)))
    );

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 5, 1)).await),
        vec![
            ("fee", 30_000, day(2024, 5, 1), day(2024, 8, 1)),
            ("usage", 30_000, day(2024, 2, 1), day(2024, 5, 1)),
        ]
    );
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 5, 1), Some(day(2024, 8, 1)))
    );

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 8, 1)).await),
        vec![
            ("fee", 30_000, day(2024, 8, 1), day(2024, 11, 1)),
            ("usage", 20_000, day(2024, 5, 1), day(2024, 8, 1)),
        ]
    );
    assert_eq!(
        billing_dates(&env, sub_id).await,
        vec![
            day(2024, 1, 1),
            day(2024, 2, 1),
            day(2024, 5, 1),
            day(2024, 8, 1)
        ]
    );
}

/// Quarterly 300.00 to monthly 29.00 at Apr 1: Apr 1 settles January to March's usage and bills
/// April; May 1 and Jun 1 each settle and bill one month.
#[tokio::test]
async fn a_quarterly_to_monthly_switch_renews_monthly_from_the_switch() {
    let (env, usage) = usage_env().await;
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let quarterly = plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(300),
    )
    .await;
    let sub_id = start(&env, quarterly, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 1, 1), day(2024, 4, 1), 3_000);
    set_usage(&usage, day(2024, 4, 1), day(2024, 5, 1), 1_000);
    set_usage(&usage, day(2024, 5, 1), day(2024, 6, 1), 500);
    switch_at_period_end(&env, sub_id, monthly).await;

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 4, 1)).await),
        vec![
            ("fee", 2_900, day(2024, 4, 1), day(2024, 5, 1)),
            ("usage", 30_000, day(2024, 1, 1), day(2024, 4, 1)),
        ]
    );
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 4, 1), Some(day(2024, 5, 1)))
    );

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 5, 1)).await),
        vec![
            ("fee", 2_900, day(2024, 5, 1), day(2024, 6, 1)),
            ("usage", 10_000, day(2024, 4, 1), day(2024, 5, 1)),
        ]
    );

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 6, 1)).await),
        vec![
            ("fee", 2_900, day(2024, 6, 1), day(2024, 7, 1)),
            ("usage", 5_000, day(2024, 5, 1), day(2024, 6, 1)),
        ]
    );
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 6, 1), Some(day(2024, 7, 1)))
    );
}

/// Monthly 29.00 to annual 3,000.00 at Feb 1, 2024: the year's fee is billed once, and the next
/// renewal is Feb 1, 2025, settling the year's usage and billing the next year.
#[tokio::test]
async fn a_monthly_to_annual_switch_renews_a_year_after_the_switch() {
    let (env, usage) = usage_env().await;
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let annual = plan(&env, "Annual", DieselBillingPeriodEnum::Annual, dec!(3000)).await;
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 1, 1), day(2024, 2, 1), 1_000);
    set_usage(&usage, day(2024, 2, 1), day(2025, 2, 1), 12_000);
    switch_at_period_end(&env, sub_id, annual).await;

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 2, 1)).await),
        vec![
            ("fee", 300_000, day(2024, 2, 1), day(2025, 2, 1)),
            ("usage", 10_000, day(2024, 1, 1), day(2024, 2, 1)),
        ]
    );
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 2, 1), Some(day(2025, 2, 1)))
    );

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2025, 2, 1)).await),
        vec![
            ("fee", 300_000, day(2025, 2, 1), day(2026, 2, 1)),
            ("usage", 120_000, day(2024, 2, 1), day(2025, 2, 1)),
        ]
    );
    assert_eq!(
        billing_dates(&env, sub_id).await,
        vec![day(2024, 1, 1), day(2024, 2, 1), day(2025, 2, 1)],
        "no renewal inside the year"
    );
}

/// Switched monthly to quarterly at Feb 1 by an earlier release, which ended the period on Mar 1
/// (fabricated). Mar 1 bills nothing and realigns the period to May 1; May 1 bills the next
/// quarter once and February to April's usage once: the fee tiles Jan–Aug without overlap.
#[tokio::test]
async fn a_subscription_left_on_the_old_grid_bills_each_quarter_once() {
    let (env, usage) = usage_env().await;
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let quarterly = plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(300),
    )
    .await;
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 1, 1), day(2024, 2, 1), 1_000);
    set_usage(&usage, day(2024, 2, 1), day(2024, 5, 1), 3_000);
    switch_at_period_end(&env, sub_id, quarterly).await;
    env.process_cycles().await;
    leave_period_end(&env, sub_id, day(2024, 3, 1)).await;

    env.process_cycles().await;
    if let Some(march) = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| invoice.billing_period_start == day(2024, 3, 1))
    {
        assert_eq!(billed(&march), vec![], "nothing is due on Mar 1");
    }
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 3, 1), Some(day(2024, 5, 1))),
        "realigned on the quarterly grid from the switch"
    );

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 5, 1)).await),
        vec![
            ("fee", 30_000, day(2024, 5, 1), day(2024, 8, 1)),
            ("usage", 30_000, day(2024, 2, 1), day(2024, 5, 1)),
        ]
    );
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 5, 1), Some(day(2024, 8, 1)))
    );
    assert_eq!(
        fee_periods(&env, sub_id).await,
        vec![
            (2_900, day(2024, 1, 1), day(2024, 2, 1)),
            (30_000, day(2024, 2, 1), day(2024, 5, 1)),
            (30_000, day(2024, 5, 1), day(2024, 8, 1)),
        ]
    );
}

/// As a release before the realignment renewed on `on`: a recurring invoice copied from the one
/// for `from`, keeping only its fees, each now billing `[on, end)`, its totals theirs and unpaid,
/// as a draft when `draft`; the period becomes `[on, end)`. Fabricated state.
async fn renew_as_earlier_release(
    env: &TestEnv,
    sub_id: SubscriptionId,
    from: NaiveDate,
    on: NaiveDate,
    end: NaiveDate,
    draft: bool,
) {
    use diesel::sql_types::{Bool, Date, Uuid as SqlUuid};
    let copied = invoice_of(env, sub_id, from).await.id;
    diesel::sql_query(
        "WITH kept AS (
             SELECT coalesce(jsonb_agg(li || jsonb_build_object(
                        'local_id', gen_random_uuid()::text,
                        'start_date', $3::date, 'end_date', $4::date)), '[]'::jsonb) AS lines,
                    coalesce(sum(coalesce((li ->> 'amount_subtotal')::bigint,
                                          (li ->> 'subtotal')::bigint)), 0) AS subtotal,
                    coalesce(sum((li ->> 'tax_amount')::bigint), 0) AS tax,
                    coalesce(sum((li ->> 'promotional_credit')::bigint), 0) AS promo
             FROM invoice src, jsonb_array_elements(src.line_items) li
             WHERE src.id = $2 AND li ->> 'metric_id' IS NULL)
         INSERT INTO invoice
         SELECT (jsonb_populate_record(i, jsonb_build_object(
             'id', $1::uuid,
             'invoice_number', i.invoice_number || '-' || to_char($3::date, 'YYYYMMDD'),
             'invoice_date', $3::date,
             'billing_period_start', $3::date,
             'cycle_index', coalesce(i.cycle_index, 0) + 1,
             'line_items', k.lines,
             'subtotal', k.subtotal,
             'subtotal_recurring', k.subtotal,
             'tax_amount', k.tax,
             'promotional_credits', k.promo,
             'discount', 0,
             'coupons', '[]'::jsonb,
             'applied_credits', 0,
             'total', k.subtotal - k.promo + k.tax,
             'amount_due', k.subtotal - k.promo + k.tax,
             'payment_status', 'UNPAID',
             'paid_at', NULL))).*
         FROM invoice i, kept k
         WHERE i.id = $2",
    )
    .bind::<SqlUuid, _>(uuid::Uuid::now_v7())
    .bind::<SqlUuid, _>(copied.as_uuid())
    .bind::<Date, _>(on)
    .bind::<Date, _>(end)
    .execute(&mut env.conn().await)
    .await
    .expect("renewal of an earlier release");
    diesel::sql_query(
        "UPDATE invoice
         SET status = 'DRAFT', invoice_number = 'draft', finalized_at = NULL, issued_at = NULL,
             payment_status = 'UNPAID'
         WHERE subscription_id = $1 AND billing_period_start = $2 AND $3",
    )
    .bind::<SqlUuid, _>(sub_id.as_uuid())
    .bind::<Date, _>(on)
    .bind::<Bool, _>(draft)
    .execute(&mut env.conn().await)
    .await
    .expect("renewal left a draft");
    use diesel_models::schema::subscription::dsl as s;
    diesel::update(s::subscription.filter(s::id.eq(sub_id)))
        .set((
            s::current_period_start.eq(on),
            s::current_period_end.eq(Some(end)),
            s::cycle_index.eq(s::cycle_index + 1),
        ))
        .execute(&mut env.conn().await)
        .await
        .expect("period of an earlier release's renewal");
}

/// Switched monthly to quarterly at Feb 1 by an earlier release, which then renewed Mar 1 for
/// [Mar 1, Jun 1) (fabricated) and kept a quarterly grid from there. Jun 1 stays on that grid:
/// it bills [Jun 1, Sep 1) once and March to May's usage once; Sep 1 the next quarter.
#[tokio::test]
async fn a_grid_an_earlier_release_renewed_on_is_kept() {
    let (env, usage) = usage_env().await;
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let quarterly = plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(300),
    )
    .await;
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 1, 1), day(2024, 2, 1), 1_000);
    set_usage(&usage, day(2024, 3, 1), day(2024, 6, 1), 2_500);
    set_usage(&usage, day(2024, 6, 1), day(2024, 9, 1), 2_000);
    switch_at_period_end(&env, sub_id, quarterly).await;
    env.process_cycles().await;
    renew_as_earlier_release(
        &env,
        sub_id,
        day(2024, 2, 1),
        day(2024, 3, 1),
        day(2024, 6, 1),
        false,
    )
    .await;

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 6, 1)).await),
        vec![
            ("fee", 30_000, day(2024, 6, 1), day(2024, 9, 1)),
            ("usage", 25_000, day(2024, 3, 1), day(2024, 6, 1)),
        ]
    );
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 6, 1), Some(day(2024, 9, 1)))
    );

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 9, 1)).await),
        vec![
            ("fee", 30_000, day(2024, 9, 1), day(2024, 12, 1)),
            ("usage", 20_000, day(2024, 6, 1), day(2024, 9, 1)),
        ]
    );
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 9, 1), Some(day(2024, 12, 1)))
    );
}

/// The same, renewed by the earlier release on Mar 1, Jun 1 and Sep 1 (fabricated): Dec 1 bills
/// [Dec 1, Mar 1) once and September to November's usage once.
#[tokio::test]
async fn a_grid_an_earlier_release_renewed_on_for_quarters_is_kept() {
    let (env, usage) = usage_env().await;
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let quarterly = plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(300),
    )
    .await;
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 9, 1), day(2024, 12, 1), 1_200);
    switch_at_period_end(&env, sub_id, quarterly).await;
    env.process_cycles().await;
    for (from, on, end) in [
        (day(2024, 2, 1), day(2024, 3, 1), day(2024, 6, 1)),
        (day(2024, 3, 1), day(2024, 6, 1), day(2024, 9, 1)),
        (day(2024, 6, 1), day(2024, 9, 1), day(2024, 12, 1)),
    ] {
        renew_as_earlier_release(&env, sub_id, from, on, end, false).await;
    }

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 12, 1)).await),
        vec![
            ("fee", 30_000, day(2024, 12, 1), day(2025, 3, 1)),
            ("usage", 12_000, day(2024, 9, 1), day(2024, 12, 1)),
        ]
    );
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 12, 1), Some(day(2025, 3, 1)))
    );
}

/// The earlier release renewed Mar 1 and left the Jun 1 renewal a draft billing [Jun 1, Sep 1)
/// (fabricated). Finalized now, it keeps that fee and settles March to May's usage.
#[tokio::test]
async fn a_draft_an_earlier_release_left_on_its_grid_settles_its_quarter() {
    let (env, usage) = usage_env().await;
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let quarterly = plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(300),
    )
    .await;
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 3, 1), day(2024, 6, 1), 2_500);
    switch_at_period_end(&env, sub_id, quarterly).await;
    env.process_cycles().await;
    renew_as_earlier_release(
        &env,
        sub_id,
        day(2024, 2, 1),
        day(2024, 3, 1),
        day(2024, 6, 1),
        false,
    )
    .await;
    renew_as_earlier_release(
        &env,
        sub_id,
        day(2024, 3, 1),
        day(2024, 6, 1),
        day(2024, 9, 1),
        true,
    )
    .await;

    let draft = invoice_of(&env, sub_id, day(2024, 6, 1)).await;
    let finalized = env
        .services()
        .finalize_invoice(Actor::System, draft.id, TENANT_ID)
        .await
        .expect("the earlier release's draft finalizes")
        .invoice;
    assert_eq!(
        billed(&finalized),
        vec![
            ("fee", 30_000, day(2024, 6, 1), day(2024, 9, 1)),
            ("usage", 25_000, day(2024, 3, 1), day(2024, 6, 1)),
        ]
    );
}

/// Bandwidth at 0.10 a unit in arrears alone, on `period`.
async fn usage_only_plan(
    env: &TestEnv,
    name: &'static str,
    period: DieselBillingPeriodEnum,
) -> PlanVersionId {
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), name, version_id)
        .components(vec![SeedComp::usage(
            PriceComponentId::new(),
            "Bandwidth",
            PRODUCT_BANDWIDTH_ID,
            METRIC_BANDWIDTH,
            PriceId::new(),
            period,
            UsagePricingModel::PerUnit { rate: dec!(0.10) },
        )])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    version_id
}

async fn cancel(
    env: &TestEnv,
    sub_id: SubscriptionId,
    at: meteroid_store::repositories::subscriptions::CancellationEffectiveAt,
) {
    env.services()
        .cancel_subscription(Actor::System, sub_id, TENANT_ID, None, at)
        .await
        .expect("cancellation scheduled");
}

/// Monthly to quarterly at Feb 1, which the earlier release renewed Mar 1 for [Mar 1, Jun 1)
/// (fabricated). The subscription is monthly 29.00 then quarterly 300.00, with bandwidth.
async fn on_an_earlier_release_grid(env: &TestEnv, quarterly_usage_only: bool) -> SubscriptionId {
    let monthly = plan(env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let quarterly = if quarterly_usage_only {
        usage_only_plan(env, "Quarterly usage", DieselBillingPeriodEnum::Quarterly).await
    } else {
        plan(
            env,
            "Quarterly",
            DieselBillingPeriodEnum::Quarterly,
            dec!(300),
        )
        .await
    };
    let sub_id = start(env, monthly, day(2024, 1, 1)).await;
    switch_at_period_end(env, sub_id, quarterly).await;
    env.process_cycles().await;
    renew_as_earlier_release(
        env,
        sub_id,
        day(2024, 2, 1),
        day(2024, 3, 1),
        day(2024, 6, 1),
        false,
    )
    .await;
    sub_id
}

/// On the earlier release's grid, cancelled on Apr 15: the final invoice settles the usage since
/// the Mar 1 renewal, [Mar 1, Apr 15), once.
#[tokio::test]
async fn a_cancellation_on_an_earlier_release_grid_settles_from_its_renewal() {
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
    let (env, usage) = usage_env().await;
    set_usage(&usage, day(2024, 3, 1), day(2024, 4, 15), 1_500);
    let sub_id = on_an_earlier_release_grid(&env, false).await;

    cancel(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(day(2024, 4, 15)),
    )
    .await;
    env.process_due_events().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 4, 15)).await),
        vec![("usage", 15_000, day(2024, 3, 1), day(2024, 4, 15))]
    );
}

/// On the earlier release's grid, cancelled at the end of its period: Jun 1 settles the quarter's
/// usage, [Mar 1, Jun 1), and bills no fee.
#[tokio::test]
async fn a_cancellation_at_the_end_of_an_earlier_release_period_settles_its_quarter() {
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
    let (env, usage) = usage_env().await;
    set_usage(&usage, day(2024, 3, 1), day(2024, 6, 1), 2_500);
    let sub_id = on_an_earlier_release_grid(&env, false).await;

    cancel(&env, sub_id, CancellationEffectiveAt::EndOfBillingPeriod).await;
    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 6, 1)).await),
        vec![("usage", 25_000, day(2024, 3, 1), day(2024, 6, 1))]
    );
}

/// The earlier release left its Mar 1 renewal a draft (fabricated), and the subscription is
/// cancelled on Mar 10 before it is issued: the final invoice settles [Mar 1, Mar 10), and the
/// draft, issued after, February's usage. Each window is billed once, whatever the order.
#[tokio::test]
async fn a_draft_of_an_earlier_release_cancelled_before_it_is_issued_bills_each_window_once() {
    use meteroid_store::domain::enums::InvoiceStatusEnum;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
    let (env, usage) = usage_env().await;
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let quarterly = plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(300),
    )
    .await;
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 2, 1), day(2024, 3, 1), 1_000);
    set_usage(&usage, day(2024, 3, 1), day(2024, 3, 10), 400);
    switch_at_period_end(&env, sub_id, quarterly).await;
    env.process_cycles().await;
    renew_as_earlier_release(
        &env,
        sub_id,
        day(2024, 2, 1),
        day(2024, 3, 1),
        day(2024, 6, 1),
        true,
    )
    .await;

    cancel(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(day(2024, 3, 10)),
    )
    .await;
    env.process_due_events().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 3, 10)).await),
        vec![("usage", 4_000, day(2024, 3, 1), day(2024, 3, 10))]
    );

    let march = invoice_of(&env, sub_id, day(2024, 3, 1)).await;
    let march = if march.status == InvoiceStatusEnum::Draft {
        env.services()
            .finalize_invoice(Actor::System, march.id, TENANT_ID)
            .await
            .expect("the earlier release's draft finalizes")
            .invoice
    } else {
        march
    };
    assert_eq!(
        billed(&march),
        vec![
            ("fee", 30_000, day(2024, 3, 1), day(2024, 6, 1)),
            ("usage", 10_000, day(2024, 2, 1), day(2024, 3, 1)),
        ]
    );
}

/// Quarterly usage alone after the switch: the earlier release's Mar 1 renewal billed nothing, so
/// no line shows its grid (fabricated). Jun 1 reads it from the period and records it; Sep 1 and
/// a cancellation on Oct 25 stay on it: each window is billed once, August included.
#[tokio::test]
async fn an_earlier_release_grid_with_an_empty_renewal_is_kept_through_a_cancellation() {
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
    let (env, usage) = usage_env().await;
    set_usage(&usage, day(2024, 1, 1), day(2024, 2, 1), 1_000);
    set_usage(&usage, day(2024, 3, 1), day(2024, 6, 1), 2_500);
    set_usage(&usage, day(2024, 6, 1), day(2024, 9, 1), 2_000);
    set_usage(&usage, day(2024, 9, 1), day(2024, 10, 25), 900);
    let sub_id = on_an_earlier_release_grid(&env, true).await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 3, 1)).await),
        vec![]
    );

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 6, 1)).await),
        vec![("usage", 25_000, day(2024, 3, 1), day(2024, 6, 1))]
    );
    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 9, 1)).await),
        vec![("usage", 20_000, day(2024, 6, 1), day(2024, 9, 1))]
    );

    cancel(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(day(2024, 10, 25)),
    )
    .await;
    env.process_due_events().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 10, 25)).await),
        vec![("usage", 9_000, day(2024, 9, 1), day(2024, 10, 25))]
    );
}

/// Switched quarterly to monthly at Apr 1 by an earlier release, which ended the period on Jul 1
/// (fabricated). The Jul 1 renewal bills the months it skipped: May, June and July's fee, and
/// April to June's usage month by month; the period realigns to [Jul 1, Aug 1).
#[tokio::test]
async fn a_subscription_left_on_the_old_grid_bills_the_months_it_skipped() {
    let (env, usage) = usage_env().await;
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let quarterly = plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(300),
    )
    .await;
    let sub_id = start(&env, quarterly, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 1, 1), day(2024, 4, 1), 3_000);
    set_usage(&usage, day(2024, 4, 1), day(2024, 5, 1), 1_000);
    set_usage(&usage, day(2024, 5, 1), day(2024, 6, 1), 800);
    set_usage(&usage, day(2024, 6, 1), day(2024, 7, 1), 700);
    switch_at_period_end(&env, sub_id, monthly).await;
    env.process_cycles().await;
    leave_period_end(&env, sub_id, day(2024, 7, 1)).await;

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 7, 1)).await),
        vec![
            ("fee", 2_900, day(2024, 5, 1), day(2024, 6, 1)),
            ("fee", 2_900, day(2024, 6, 1), day(2024, 7, 1)),
            ("fee", 2_900, day(2024, 7, 1), day(2024, 8, 1)),
            ("usage", 7_000, day(2024, 6, 1), day(2024, 7, 1)),
            ("usage", 8_000, day(2024, 5, 1), day(2024, 6, 1)),
            ("usage", 10_000, day(2024, 4, 1), day(2024, 5, 1)),
        ]
    );
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 7, 1), Some(day(2024, 8, 1)))
    );
    assert_eq!(
        fee_periods(&env, sub_id).await,
        vec![
            (30_000, day(2024, 1, 1), day(2024, 4, 1)),
            (2_900, day(2024, 4, 1), day(2024, 5, 1)),
            (2_900, day(2024, 5, 1), day(2024, 6, 1)),
            (2_900, day(2024, 6, 1), day(2024, 7, 1)),
            (2_900, day(2024, 7, 1), day(2024, 8, 1)),
        ]
    );
}

/// A fixed fee of `fee` per period billed in arrears and bandwidth at 0.10 a unit in arrears.
async fn arrears_plan(
    env: &TestEnv,
    name: &'static str,
    period: DieselBillingPeriodEnum,
    fee: Decimal,
) -> PlanVersionId {
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), name, version_id)
        .components(vec![
            SeedComp::extra_recurring(
                PriceComponentId::new(),
                "Arrears Fee",
                PRODUCT_RECURRING_ARREARS_ID,
                PriceId::new(),
                period.clone(),
                fee,
                1,
                BillingType::Arrears,
            ),
            SeedComp::usage(
                PriceComponentId::new(),
                "Bandwidth",
                PRODUCT_BANDWIDTH_ID,
                METRIC_BANDWIDTH,
                PriceId::new(),
                period,
                UsagePricingModel::PerUnit { rate: dec!(0.10) },
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    version_id
}

/// Seats at 10.00 (monthly) or 30.00 (quarterly) and a capacity of 50.00 (or 150.00) including
/// 1,000 (or 3,000) units of bandwidth, overage at 0.10.
async fn capacity_and_seats_plan(
    env: &TestEnv,
    name: &'static str,
    period: DieselBillingPeriodEnum,
) -> PlanVersionId {
    let quarterly = matches!(period, DieselBillingPeriodEnum::Quarterly);
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), name, version_id)
        .components(vec![
            SeedComp::slot(
                PriceComponentId::new(),
                "Seats",
                PRODUCT_SEATS_ID,
                PriceId::new(),
                period.clone(),
                if quarterly { dec!(30) } else { dec!(10) },
            ),
            SeedComp::capacity(
                PriceComponentId::new(),
                "Capacity",
                PRODUCT_CAPACITY_ID,
                METRIC_BANDWIDTH,
                PriceId::new(),
                period,
                if quarterly { dec!(150) } else { dec!(50) },
                if quarterly { 3_000 } else { 1_000 },
                dec!(0.10),
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    version_id
}

/// A product's billed lines on an invoice: amount and service period.
fn lines_of(invoice: &Invoice, product: ProductId) -> Vec<(i64, NaiveDate, NaiveDate)> {
    let mut lines: Vec<_> = invoice
        .line_items
        .iter()
        .filter(|l| l.product_id == Some(product) && l.amount_subtotal != 0)
        .map(|l| (l.amount_subtotal, l.start_date, l.end_date))
        .collect();
    lines.sort();
    lines
}

/// Monthly to quarterly at Feb 1, back to monthly at May 1: the quarter's usage is billed once,
/// on May 1 over [Feb 1, May 1), on the quarterly grid the closed component ran on; June is monthly.
#[tokio::test]
async fn a_second_change_of_cadence_settles_the_quarter_it_closes_once() {
    let (env, usage) = usage_env().await;
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let quarterly = plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(300),
    )
    .await;
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 1, 1), day(2024, 2, 1), 1_000);
    set_usage(&usage, day(2024, 2, 1), day(2024, 5, 1), 3_000);
    set_usage(&usage, day(2024, 5, 1), day(2024, 6, 1), 500);
    switch_at_period_end(&env, sub_id, quarterly).await;
    env.process_cycles().await;
    switch_at_period_end(&env, sub_id, monthly).await;

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 5, 1)).await),
        vec![
            ("fee", 2_900, day(2024, 5, 1), day(2024, 6, 1)),
            ("usage", 30_000, day(2024, 2, 1), day(2024, 5, 1)),
        ]
    );
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 5, 1), Some(day(2024, 6, 1)))
    );

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 6, 1)).await),
        vec![
            ("fee", 2_900, day(2024, 6, 1), day(2024, 7, 1)),
            ("usage", 5_000, day(2024, 5, 1), day(2024, 6, 1)),
        ]
    );
    let mut usage_lines: Vec<_> = env
        .get_invoices(sub_id)
        .await
        .iter()
        .flat_map(billed)
        .filter(|(kind, ..)| *kind == "usage")
        .map(|(_, amount, from, to)| (from, to, amount))
        .collect();
    usage_lines.sort();
    assert_eq!(
        usage_lines,
        vec![
            (day(2024, 1, 1), day(2024, 2, 1), 10_000),
            (day(2024, 2, 1), day(2024, 5, 1), 30_000),
            (day(2024, 5, 1), day(2024, 6, 1), 5_000),
        ],
        "each period's usage billed once"
    );
}

/// Monthly (31.00 in arrears) to quarterly (910.00 in arrears) immediately on Mar 15, 2024: Apr 1
/// bills the old fee for [Mar 1, Mar 15), 3100 × 14/31 = 1400, and the new one for its 17 days
/// of the quarter [Jan 1, Apr 1), 91000 × 17/91 = 17000; the period runs to Jul 1.
#[tokio::test]
async fn an_immediate_change_to_a_longer_cadence_prorates_the_stub_on_the_new_cadence() {
    let (env, usage) = usage_env().await;
    let monthly = arrears_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(31)).await;
    let quarterly = arrears_plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(910),
    )
    .await;
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 3, 1), day(2024, 3, 15), 140);
    set_usage(&usage, day(2024, 3, 15), day(2024, 4, 1), 170);
    env.process_cycles().await;
    env.process_cycles().await;
    assert_eq!(period(&env, sub_id).await.0, day(2024, 3, 1));
    env.services()
        .apply_plan_change_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            quarterly,
            vec![],
            day(2024, 3, 15),
        )
        .await
        .expect("an immediate change without a minimum");

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 4, 1)).await),
        vec![
            ("fee", 1_400, day(2024, 3, 1), day(2024, 3, 15)),
            ("fee", 17_000, day(2024, 3, 15), day(2024, 4, 1)),
            ("usage", 1_400, day(2024, 3, 1), day(2024, 3, 15)),
            ("usage", 1_700, day(2024, 3, 15), day(2024, 4, 1)),
        ]
    );
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 4, 1), Some(day(2024, 7, 1)))
    );
}

/// Quarterly (910.00 in arrears) to monthly (31.00 in arrears) immediately on Mar 15, 2024: Apr 1
/// bills the old fee for [Jan 1, Mar 15), 91000 × 74/91 = 74000, and the new one for its 17 days
/// of the month [Mar 1, Apr 1), 3100 × 17/31 = 1700; the period runs to May 1.
#[tokio::test]
async fn an_immediate_change_to_a_shorter_cadence_prorates_the_stub_on_the_new_cadence() {
    let (env, usage) = usage_env().await;
    let monthly = arrears_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(31)).await;
    let quarterly = arrears_plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(910),
    )
    .await;
    let sub_id = start(&env, quarterly, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 1, 1), day(2024, 3, 15), 740);
    set_usage(&usage, day(2024, 3, 15), day(2024, 4, 1), 170);
    env.services()
        .apply_plan_change_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            monthly,
            vec![],
            day(2024, 3, 15),
        )
        .await
        .expect("an immediate change without a minimum");

    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 4, 1)).await),
        vec![
            ("fee", 1_700, day(2024, 3, 15), day(2024, 4, 1)),
            ("fee", 74_000, day(2024, 1, 1), day(2024, 3, 15)),
            ("usage", 1_700, day(2024, 3, 15), day(2024, 4, 1)),
            ("usage", 7_400, day(2024, 1, 1), day(2024, 3, 15)),
        ]
    );
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 4, 1), Some(day(2024, 5, 1)))
    );
}

/// Quarterly to monthly at Apr 1 with an end date on Jun 1: April and May are periods of their
/// own, each fee billed; the end date cuts the realigned grid, and Jun 1 settles May's usage.
#[tokio::test]
async fn an_end_date_cuts_the_period_of_the_new_cadence() {
    let (env, usage) = usage_env().await;
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let quarterly = plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(300),
    )
    .await;
    let sub_id = subscription()
        .plan_version(quarterly)
        .start_date(day(2024, 1, 1))
        .end_date(day(2024, 6, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    set_usage(&usage, day(2024, 1, 1), day(2024, 4, 1), 3_000);
    set_usage(&usage, day(2024, 4, 1), day(2024, 5, 1), 1_000);
    set_usage(&usage, day(2024, 5, 1), day(2024, 6, 1), 500);
    switch_at_period_end(&env, sub_id, monthly).await;

    env.process_cycles().await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 4, 1), Some(day(2024, 5, 1)))
    );
    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 5, 1)).await),
        vec![
            ("fee", 2_900, day(2024, 5, 1), day(2024, 6, 1)),
            ("usage", 10_000, day(2024, 4, 1), day(2024, 5, 1)),
        ]
    );
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 5, 1), Some(day(2024, 6, 1)))
    );
    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 6, 1)).await),
        vec![("usage", 5_000, day(2024, 5, 1), day(2024, 6, 1))]
    );
    assert_eq!(
        fee_periods(&env, sub_id).await,
        vec![
            (30_000, day(2024, 1, 1), day(2024, 4, 1)),
            (2_900, day(2024, 4, 1), day(2024, 5, 1)),
            (2_900, day(2024, 5, 1), day(2024, 6, 1)),
        ]
    );
}

/// A monthly 50.00 minimum after a quarterly to monthly switch at Apr 1 left ending Jul 1
/// (fabricated): Jul 1 trues up three whole months, 3 × 5000 − 3 × 2900 = 6300, over [Apr 1,
/// Jul 1); by days it would be 5000 × 91/30.
#[tokio::test]
async fn a_repair_trues_up_each_skipped_month_at_the_monthly_minimum() {
    let (env, _usage) = usage_env().await;
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let quarterly = plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(300),
    )
    .await;
    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(
            &diesel_models::plan_version_adjustments::PlanVersionAdjustmentRowNew {
                kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                plan_version_id: monthly,
                amount_subunit: 5_000,
                includes_all_components: true,
            },
        )
        .execute(&mut env.conn().await)
        .await
        .expect("monthly minimum");
    let sub_id = start(&env, quarterly, day(2024, 1, 1)).await;
    switch_at_period_end(&env, sub_id, monthly).await;
    env.process_cycles().await;
    leave_period_end(&env, sub_id, day(2024, 7, 1)).await;

    env.process_cycles().await;
    let july = invoice_of(&env, sub_id, day(2024, 7, 1)).await;
    let shortfall: Vec<_> = july
        .line_items
        .iter()
        .filter(|l| l.kind == LineItemKind::MinimumShortfall)
        .map(|l| (l.amount_subtotal, l.start_date, l.end_date))
        .collect();
    assert_eq!(shortfall, vec![(6_300, day(2024, 4, 1), day(2024, 7, 1))]);
}

/// Quarterly to monthly at Apr 1, left ending Jul 1 (fabricated), with 1,000 units a month and
/// 1,000 included a month: no overage. Seats are 3 from May 1 and 5 from Jun 1: each catch-up
/// month bills its own count, 10.00 a seat.
#[tokio::test]
async fn a_repair_bills_each_skipped_month_with_its_own_capacity_and_seats() {
    let (env, usage) = usage_env().await;
    let monthly = capacity_and_seats_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly).await;
    let quarterly =
        capacity_and_seats_plan(&env, "Quarterly", DieselBillingPeriodEnum::Quarterly).await;
    let sub_id = start(&env, quarterly, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 4, 1), day(2024, 7, 1), 3_000);
    set_usage(&usage, day(2024, 4, 1), day(2024, 5, 1), 1_000);
    set_usage(&usage, day(2024, 5, 1), day(2024, 6, 1), 1_000);
    set_usage(&usage, day(2024, 6, 1), day(2024, 7, 1), 1_000);
    switch_at_period_end(&env, sub_id, monthly).await;
    env.process_cycles().await;
    leave_period_end(&env, sub_id, day(2024, 7, 1)).await;

    // Seat counts from May 1 and Jun 1, as checkpoints of the seat history.
    {
        use diesel_models::schema::slot_transaction::dsl as st;
        let mut conn = env.conn().await;
        let unit: String = st::slot_transaction
            .filter(st::subscription_id.eq(sub_id))
            .select(st::unit)
            .first(&mut conn)
            .await
            .expect("the seats' unit");
        for (on, seats) in [(day(2024, 5, 1), 3), (day(2024, 6, 1), 5)] {
            let at = on.and_time(chrono::NaiveTime::MIN);
            diesel::sql_query(
                "INSERT INTO slot_transaction (id, subscription_id, delta, prev_active_slots, \
                 effective_at, transaction_at, unit, status) \
                 VALUES ($1, $2, 0, $3, $4, $4, $5, 'ACTIVE')",
            )
            .bind::<diesel::sql_types::Uuid, _>(uuid::Uuid::now_v7())
            .bind::<diesel::sql_types::Uuid, _>(sub_id)
            .bind::<diesel::sql_types::Integer, _>(seats)
            .bind::<diesel::sql_types::Timestamp, _>(at)
            .bind::<diesel::sql_types::VarChar, _>(unit.clone())
            .execute(&mut conn)
            .await
            .expect("seat checkpoint");
        }
    }

    env.process_cycles().await;
    let july = invoice_of(&env, sub_id, day(2024, 7, 1)).await;
    assert_eq!(
        lines_of(&july, PRODUCT_SEATS_ID),
        vec![
            (3_000, day(2024, 5, 1), day(2024, 6, 1)),
            (5_000, day(2024, 6, 1), day(2024, 7, 1)),
            (5_000, day(2024, 7, 1), day(2024, 8, 1)),
        ]
    );
    assert_eq!(
        lines_of(&july, PRODUCT_CAPACITY_ID),
        vec![
            (5_000, day(2024, 5, 1), day(2024, 6, 1)),
            (5_000, day(2024, 6, 1), day(2024, 7, 1)),
            (5_000, day(2024, 7, 1), day(2024, 8, 1)),
        ],
        "each month's capacity, and no overage: 1,000 used of 1,000 included each month"
    );
}
