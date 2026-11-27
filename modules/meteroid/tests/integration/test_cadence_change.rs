//! A plan switch at a period's end that changes the cadence (monthly to quarterly, quarterly to
//! monthly, monthly to annual): periods after it run on the new cadence from the switch, each fee
//! is billed once per period and every period's usage is billed once.
use std::sync::Arc;

use chrono::NaiveDate;
use common_domain::actor::Actor;
use common_domain::ids::{
    AddOnId, BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId, ProductId, SubscriptionId,
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
/// (fabricated). Mar 1 bills nothing and the facts put the period back on the quarter from the
/// switch; May 1 bills the next quarter once and February to April's usage once: the fee tiles
/// Jan–Aug without overlap.
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
        (day(2024, 2, 1), Some(day(2024, 5, 1))),
        "the quarter from the switch"
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
         SET status = 'DRAFT', invoice_number = 'draft', finalized_at = NULL,
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

    // The cancellation is requested on the day it ends from.
    env.set_now(day(2024, 4, 15).and_hms_opt(9, 0, 0).unwrap().and_utc());
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

    // The cancellation is requested inside the period it ends.
    env.set_now(day(2024, 5, 15).and_hms_opt(9, 0, 0).unwrap().and_utc());
    cancel(&env, sub_id, CancellationEffectiveAt::EndOfBillingPeriod).await;
    env.set_now(day(2024, 6, 1).and_hms_opt(9, 0, 0).unwrap().and_utc());
    env.process_cycles().await;
    assert_eq!(
        billed(&invoice_of(&env, sub_id, day(2024, 6, 1)).await),
        vec![("usage", 25_000, day(2024, 3, 1), day(2024, 6, 1))]
    );
}

/// Monthly 29.00 with bandwidth from Jan 1, 2024; the controller is held back past the Feb 1
/// renewal, so the stored period is still [Jan 1, Feb 1) when the subscription is cancelled at
/// the end of its period on Feb 10. The period in force then is [Feb 1, Mar 1): the cancellation
/// ends it on Mar 1, never backdated to Feb 1. The Feb 1 renewal bills February's fee and
/// January's usage, issued on Feb 10 when the controller catches up, and Mar 1 February's usage:
/// no final invoice, nothing given back.
/// Rules: R-084, R-016
#[tokio::test]
async fn a_cancellation_at_the_period_end_behind_a_held_renewal_ends_the_period_in_force() {
    use diesel_models::enums::SubscriptionStatusEnum;
    use diesel_models::subscription_facts::SubscriptionEndingRow;
    use meteroid_store::domain::enums::InvoiceType;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
    let (env, usage) = usage_env().await;
    let at = |m: u32, d: u32| day(2024, m, d).and_hms_opt(9, 0, 0).unwrap().and_utc();
    env.set_now(at(1, 1));
    set_usage(&usage, day(2024, 1, 1), day(2024, 2, 1), 1_000);
    set_usage(&usage, day(2024, 2, 1), day(2024, 3, 1), 400);
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    env.process_cycles().await;

    env.set_now(at(2, 10));
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 1, 1), Some(day(2024, 2, 1)))
    );
    cancel(&env, sub_id, CancellationEffectiveAt::EndOfBillingPeriod).await;
    let endings = SubscriptionEndingRow::list_of(&mut env.conn().await, sub_id)
        .await
        .expect("the endings");
    assert_eq!(
        endings.iter().map(|ending| ending.at).collect::<Vec<_>>(),
        vec![day(2024, 3, 1).and_time(chrono::NaiveTime::MIN)]
    );
    assert_ne!(
        env.get_subscription(sub_id).await.status,
        SubscriptionStatusEnum::Cancelled
    );

    env.process_cycles().await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 2, 1), Some(day(2024, 3, 1)))
    );
    env.set_now(at(3, 1));
    for _ in 0..2 {
        env.process_cycles().await;
    }

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-02-01: Bandwidth 2024-01-01..2024-02-01 10000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
            "Recurring 2024-03-01: Bandwidth 2024-02-01..2024-03-01 4000",
        ]
    );
    assert_eq!(
        statuses(&env, sub_id)
            .await
            .into_iter()
            .map(|(kind, date, _)| (kind, date))
            .collect::<Vec<_>>(),
        vec![
            (InvoiceType::Recurring, day(2024, 1, 1)),
            (InvoiceType::Recurring, day(2024, 2, 10)),
            (InvoiceType::Recurring, day(2024, 3, 1)),
        ]
    );
    assert_eq!(
        env.get_subscription(sub_id).await.status,
        SubscriptionStatusEnum::Cancelled
    );
    assert_eq!(period(&env, sub_id).await, (day(2024, 3, 1), None));
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

    // The cancellation is requested on the day it ends from.
    env.set_now(day(2024, 3, 10).and_hms_opt(9, 0, 0).unwrap().and_utc());
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
    // The cancellation is requested on the day it ends from.
    env.set_now(day(2024, 10, 25).and_hms_opt(9, 0, 0).unwrap().and_utc());
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
/// (fabricated). Caught up one boundary at a time, May 1, Jun 1 and Jul 1 each renew their month
/// and bill the month before's usage; the period realigns to [Jul 1, Aug 1).
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
    let mut renewals = Vec::new();
    for on in [day(2024, 5, 1), day(2024, 6, 1), day(2024, 7, 1)] {
        renewals.push(billed(&invoice_of(&env, sub_id, on).await));
    }
    assert_eq!(
        renewals,
        vec![
            vec![
                ("fee", 2_900, day(2024, 5, 1), day(2024, 6, 1)),
                ("usage", 10_000, day(2024, 4, 1), day(2024, 5, 1)),
            ],
            vec![
                ("fee", 2_900, day(2024, 6, 1), day(2024, 7, 1)),
                ("usage", 8_000, day(2024, 5, 1), day(2024, 6, 1)),
            ],
            vec![
                ("fee", 2_900, day(2024, 7, 1), day(2024, 8, 1)),
                ("usage", 7_000, day(2024, 6, 1), day(2024, 7, 1)),
            ],
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
    // Each month is billed once, on its own renewal.
    let mut conn = env.conn().await;
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(
        crate::helpers::facts::owed_on_written_documents(
            &mut conn, TENANT_ID, sub_id, &invoices, false
        )
        .await,
        vec![]
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
/// (fabricated): each renewal caught up trues up its whole month, 5000 − 2900 = 2100, three in
/// all for [Apr 1, Jul 1); by days it would be 5000 × 91/30.
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
    let mut shortfall = Vec::new();
    for on in [day(2024, 5, 1), day(2024, 6, 1), day(2024, 7, 1)] {
        let renewal = invoice_of(&env, sub_id, on).await;
        shortfall.extend(
            renewal
                .line_items
                .iter()
                .filter(|l| l.kind == LineItemKind::MinimumShortfall)
                .map(|l| (l.amount_subtotal, l.start_date, l.end_date)),
        );
    }
    assert_eq!(
        shortfall,
        vec![
            (2_100, day(2024, 4, 1), day(2024, 5, 1)),
            (2_100, day(2024, 5, 1), day(2024, 6, 1)),
            (2_100, day(2024, 6, 1), day(2024, 7, 1)),
        ]
    );
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
                 effective_at, transaction_at, unit, status, managed_quantity_id) \
                 SELECT $1, $2, 0, $3, $4, $4, $5, 'ACTIVE', q.id \
                 FROM managed_quantity q JOIN subscription s ON s.tenant_id = q.tenant_id \
                 WHERE s.id = $2 AND q.code = $5",
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
    let mut renewals = Vec::new();
    for on in [day(2024, 5, 1), day(2024, 6, 1), day(2024, 7, 1)] {
        renewals.push(invoice_of(&env, sub_id, on).await);
    }
    let lines = |product| -> Vec<_> {
        renewals
            .iter()
            .flat_map(|renewal| lines_of(renewal, product))
            .collect()
    };
    assert_eq!(
        lines(PRODUCT_SEATS_ID),
        vec![
            (3_000, day(2024, 5, 1), day(2024, 6, 1)),
            (5_000, day(2024, 6, 1), day(2024, 7, 1)),
            (5_000, day(2024, 7, 1), day(2024, 8, 1)),
        ]
    );
    assert_eq!(
        lines(PRODUCT_CAPACITY_ID),
        vec![
            (5_000, day(2024, 5, 1), day(2024, 6, 1)),
            (5_000, day(2024, 6, 1), day(2024, 7, 1)),
            (5_000, day(2024, 7, 1), day(2024, 8, 1)),
        ],
        "each month's capacity, and no overage: 1,000 used of 1,000 included each month"
    );
}

/// A single Platform Fee of `fee` per period in advance.
async fn fee_plan(
    env: &TestEnv,
    name: &'static str,
    period: DieselBillingPeriodEnum,
    fee: Decimal,
) -> PlanVersionId {
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), name, version_id)
        .components(vec![SeedComp::rate(
            PriceComponentId::new(),
            "Platform Fee",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            period,
            fee,
        )])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    version_id
}

async fn change_now(env: &TestEnv, sub_id: SubscriptionId, to: PlanVersionId, on: NaiveDate) {
    env.services()
        .apply_plan_change_immediate_at(Actor::System, sub_id, TENANT_ID, to, vec![], on)
        .await
        .expect("an immediate plan change");
}

/// Every document of the subscription in the order issued, one entry per line: each invoice's
/// lines, then those of the notes crediting it.
async fn documents(env: &TestEnv, sub_id: SubscriptionId) -> Vec<String> {
    use meteroid_store::repositories::CreditNoteInterface;
    let mut documents = Vec::new();
    for invoice in env.get_invoices(sub_id).await {
        let of = format!(
            "{:?} {}",
            invoice.invoice_type, invoice.billing_period_start
        );
        for line in &invoice.line_items {
            documents.push(format!(
                "{of}: {} {}..{} {}",
                line.name, line.start_date, line.end_date, line.amount_subtotal
            ));
        }
        for note in env
            .store()
            .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
            .await
            .expect("credit notes")
        {
            for line in &note.line_items {
                documents.push(format!(
                    "note on {of}: {} {}..{} {}",
                    line.name, line.start_date, line.end_date, line.amount_subtotal
                ));
            }
        }
    }
    documents
}

/// Each invoice's type, date and status, in the order written.
async fn statuses(
    env: &TestEnv,
    sub_id: SubscriptionId,
) -> Vec<(
    meteroid_store::domain::enums::InvoiceType,
    NaiveDate,
    meteroid_store::domain::enums::InvoiceStatusEnum,
)> {
    env.get_invoices(sub_id)
        .await
        .into_iter()
        .map(|invoice| (invoice.invoice_type, invoice.invoice_date, invoice.status))
        .collect()
}

/// Starter (29.00 fee, 10.00 seats, monthly from Jan 1) to a yearly 290.00 fee on Jan 15, 2024.
/// The old fees give back 17 of January's 31 days: 2900 × 17/31 = 1590, 1000 × 17/31 = 548. The
/// year's grid starts on the next monthly renewal, Feb 1 (R-084), so the stub is the 17 days to it
/// of the year [2023-02-01, 2024-02-01): 29000 × 17/365 = 1351 (R-079). Feb 1 bills the year.
#[tokio::test]
async fn an_immediate_change_to_annual_bills_the_stub_to_the_next_renewal() {
    let (env, _) = usage_env().await;
    let annual = fee_plan(&env, "Annual", DieselBillingPeriodEnum::Annual, dec!(290)).await;
    let sub_id = start(&env, PLAN_VERSION_STARTER_ID, day(2024, 1, 1)).await;
    change_now(&env, sub_id, annual, day(2024, 1, 15)).await;
    env.process_cycles().await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 2, 1), Some(day(2025, 2, 1)))
    );
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "note on Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 -1590",
            "note on Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 -548",
            "Adjustment 2024-01-15: Platform Fee (prorated) 2024-01-15..2024-02-01 1351",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2025-02-01 29000",
            "Recurring 2025-02-01: Platform Fee 2025-02-01..2026-02-01 29000",
        ]
    );
}

/// The same change on Feb 1, after February's invoice: Feb 1 is a boundary of the monthly grid,
/// so the year starts there and is billed whole, 29000 over [2024-02-01, 2025-02-01); February's
/// fees are given back whole. The period is that year: the next renewal is Feb 1, 2025, with
/// nothing billed on Mar 1.
#[tokio::test]
async fn an_immediate_change_to_annual_on_a_renewal_bills_the_whole_year() {
    let (env, _) = usage_env().await;
    let annual = fee_plan(&env, "Annual", DieselBillingPeriodEnum::Annual, dec!(290)).await;
    let sub_id = start(&env, PLAN_VERSION_STARTER_ID, day(2024, 1, 1)).await;
    env.process_cycles().await;
    assert_eq!(period(&env, sub_id).await.0, day(2024, 2, 1));
    change_now(&env, sub_id, annual, day(2024, 2, 1)).await;
    // The change's period is the year it opens: no renewal on Mar 1.
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 2, 1), Some(day(2025, 2, 1)))
    );
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 1000",
            "note on Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 -2900",
            "note on Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 -1000",
            "Adjustment 2024-02-01: Platform Fee 2024-02-01..2025-02-01 29000",
            "Recurring 2025-02-01: Platform Fee 2025-02-01..2026-02-01 29000",
        ]
    );
}

/// Monthly 29.00 to quarterly 90.00 on Feb 15, 2024, February billed. The fee gives back 15 of
/// February's 29 days: 2900 × 15/29 = 1500. The quarters run from the next renewal, Mar 1
/// (R-084): the stub is 15 days of [2023-12-01, 2024-03-01), 91 days, 9000 × 15/91 = 1484.
#[tokio::test]
async fn an_immediate_change_to_quarterly_bills_the_stub_to_the_next_renewal() {
    let (env, _) = usage_env().await;
    let monthly = fee_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let quarterly = fee_plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(90),
    )
    .await;
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    env.process_cycles().await;
    change_now(&env, sub_id, quarterly, day(2024, 2, 15)).await;
    env.process_cycles().await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 3, 1), Some(day(2024, 6, 1)))
    );
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
            "note on Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 -1500",
            "Adjustment 2024-02-15: Platform Fee (prorated) 2024-02-15..2024-03-01 1484",
            "Recurring 2024-03-01: Platform Fee 2024-03-01..2024-06-01 9000",
            "Recurring 2024-06-01: Platform Fee 2024-06-01..2024-09-01 9000",
        ]
    );
}

/// Yearly 290.00 to monthly 29.00 on Mar 15, 2024. The year gives back its 292 days left of 366:
/// 29000 × 292/366 = 23137. The monthly grid runs from the change's first renewal, Apr 1 (R-084),
/// before it as after it: the stub is [Mar 15, Apr 1) of March, 2900 × 17/31 = 1590, then a month
/// each renewal.
#[tokio::test]
async fn an_immediate_change_from_annual_to_monthly_bills_each_month() {
    let (env, _) = usage_env().await;
    let monthly = fee_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let annual = fee_plan(&env, "Annual", DieselBillingPeriodEnum::Annual, dec!(290)).await;
    let sub_id = start(&env, annual, day(2024, 1, 1)).await;
    change_now(&env, sub_id, monthly, day(2024, 3, 15)).await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 3, 1), Some(day(2024, 4, 1))),
        "March on the monthly grid, within the year"
    );
    env.process_cycles().await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 4, 1), Some(day(2024, 5, 1)))
    );
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2025-01-01 29000",
            "note on Recurring 2024-01-01: Platform Fee 2024-01-01..2025-01-01 -23137",
            "Adjustment 2024-03-15: Platform Fee (prorated) 2024-03-15..2024-04-01 1590",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 2900",
            "Recurring 2024-05-01: Platform Fee 2024-05-01..2024-06-01 2900",
        ]
    );
}

/// Every line with an amount over the subscription's documents, sorted: the lines of one invoice
/// in any order.
async fn billed_documents(env: &TestEnv, sub_id: SubscriptionId) -> Vec<String> {
    let mut lines: Vec<String> = documents(env, sub_id)
        .await
        .into_iter()
        .filter(|line| !line.ends_with(" 0"))
        .collect();
    lines.sort();
    lines
}

/// Yearly (366.00 in arrears, 0.10 a unit) to monthly (31.00 in arrears) on Mar 15, 2024: Apr 1
/// settles the year to the change, 36600 × 74/366 = 7400 and 740 units = 7400, and the stub, 3100 ×
/// 17/31 = 1700 and 170 units = 1700 (R-011). Jan 1, 2025, the year's end, bills December alone.
#[tokio::test]
async fn an_immediate_change_from_annual_to_monthly_settles_the_year_in_arrears_once() {
    let (env, usage) = usage_env().await;
    let annual = arrears_plan(&env, "Annual", DieselBillingPeriodEnum::Annual, dec!(366)).await;
    let monthly = arrears_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(31)).await;
    let sub_id = start(&env, annual, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 1, 1), day(2024, 3, 15), 740);
    set_usage(&usage, day(2024, 3, 15), day(2024, 4, 1), 170);
    set_usage(&usage, day(2024, 4, 1), day(2024, 5, 1), 300);
    change_now(&env, sub_id, monthly, day(2024, 3, 15)).await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 3, 1), Some(day(2024, 4, 1)))
    );
    for _ in 0..11 {
        env.process_cycles().await;
    }
    assert_eq!(
        period(&env, sub_id).await,
        (day(2025, 2, 1), Some(day(2025, 3, 1)))
    );

    let mut expected = vec![
        "Recurring 2024-04-01: Arrears Fee 2024-01-01..2024-03-15 7400".to_string(),
        "Recurring 2024-04-01: Arrears Fee 2024-03-15..2024-04-01 1700".to_string(),
        "Recurring 2024-04-01: Bandwidth 2024-01-01..2024-03-15 7400".to_string(),
        "Recurring 2024-04-01: Bandwidth 2024-03-15..2024-04-01 1700".to_string(),
        "Recurring 2024-05-01: Bandwidth 2024-04-01..2024-05-01 3000".to_string(),
    ];
    let mut month = day(2024, 4, 1);
    while month < day(2025, 2, 1) {
        let next = month.checked_add_months(chrono::Months::new(1)).unwrap();
        expected.push(format!(
            "Recurring {next}: Arrears Fee {month}..{next} 3100"
        ));
        month = next;
    }
    expected.sort();
    assert_eq!(billed_documents(&env, sub_id).await, expected);
}

/// The same change lived day by day, so the stored state is checked against the facts at each
/// invoice: March on the monthly grid after the change, then a month from each renewal.
#[tokio::test]
async fn an_immediate_change_from_annual_to_monthly_renews_monthly_on_the_clock() {
    let (env, usage) = usage_env().await;
    let annual = arrears_plan(&env, "Annual", DieselBillingPeriodEnum::Annual, dec!(366)).await;
    let monthly = arrears_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(31)).await;
    env.set_now(day(2024, 1, 1).and_hms_opt(9, 0, 0).unwrap().and_utc());
    let sub_id = start(&env, annual, day(2024, 1, 1)).await;
    set_usage(&usage, day(2024, 1, 1), day(2024, 3, 15), 740);
    set_usage(&usage, day(2024, 3, 15), day(2024, 4, 1), 170);
    set_usage(&usage, day(2024, 4, 1), day(2024, 5, 1), 300);
    env.set_now(day(2024, 3, 15).and_hms_opt(9, 0, 0).unwrap().and_utc());
    change_now(&env, sub_id, monthly, day(2024, 3, 15)).await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 3, 1), Some(day(2024, 4, 1)))
    );
    env.run_until(day(2024, 5, 2)).await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 5, 1), Some(day(2024, 6, 1)))
    );

    assert_eq!(
        billed_documents(&env, sub_id).await,
        vec![
            "Recurring 2024-04-01: Arrears Fee 2024-01-01..2024-03-15 7400",
            "Recurring 2024-04-01: Arrears Fee 2024-03-15..2024-04-01 1700",
            "Recurring 2024-04-01: Bandwidth 2024-01-01..2024-03-15 7400",
            "Recurring 2024-04-01: Bandwidth 2024-03-15..2024-04-01 1700",
            "Recurring 2024-05-01: Arrears Fee 2024-04-01..2024-05-01 3100",
            "Recurring 2024-05-01: Bandwidth 2024-04-01..2024-05-01 3000",
        ]
    );
}

/// Yearly 290.00 to quarterly 90.00 on Mar 15, 2024. The year gives back 29000 × 292/366 = 23137.
/// The quarters fall on the year's grid (R-084): the stub is 17 days of [Jan 1, Apr 1), 91 days,
/// 9000 × 17/91 = 1681, and Apr 1 renews a quarter.
#[tokio::test]
async fn an_immediate_change_from_annual_to_quarterly_bills_each_quarter() {
    let (env, _) = usage_env().await;
    let annual = fee_plan(&env, "Annual", DieselBillingPeriodEnum::Annual, dec!(290)).await;
    let quarterly = fee_plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(90),
    )
    .await;
    let sub_id = start(&env, annual, day(2024, 1, 1)).await;
    change_now(&env, sub_id, quarterly, day(2024, 3, 15)).await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 1, 1), Some(day(2024, 4, 1)))
    );
    env.process_cycles().await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 4, 1), Some(day(2024, 7, 1)))
    );
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2025-01-01 29000",
            "note on Recurring 2024-01-01: Platform Fee 2024-01-01..2025-01-01 -23137",
            "Adjustment 2024-03-15: Platform Fee (prorated) 2024-03-15..2024-04-01 1681",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-07-01 9000",
            "Recurring 2024-07-01: Platform Fee 2024-07-01..2024-10-01 9000",
        ]
    );
}

/// Quarterly 90.00 to monthly 29.00 on Feb 10, 2024. The quarter [Jan 1, Apr 1) gives back its
/// 51 days left of 91: 9000 × 51/91 = 5044. The stub is 20 of February's 29 days, 2900 × 20/29 =
/// 2000; Mar 1 renews a month, so March is billed.
#[tokio::test]
async fn an_immediate_change_from_quarterly_to_monthly_bills_each_month() {
    let (env, _) = usage_env().await;
    let quarterly = fee_plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(90),
    )
    .await;
    let monthly = fee_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let sub_id = start(&env, quarterly, day(2024, 1, 1)).await;
    change_now(&env, sub_id, monthly, day(2024, 2, 10)).await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 2, 1), Some(day(2024, 3, 1)))
    );
    env.process_cycles().await;
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-04-01 9000",
            "note on Recurring 2024-01-01: Platform Fee 2024-01-01..2024-04-01 -5044",
            "Adjustment 2024-02-10: Platform Fee (prorated) 2024-02-10..2024-03-01 2000",
            "Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 2900",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 2900",
        ]
    );
}

/// Yearly 290.00 to monthly 29.00 on Jan 1, 2025, after that day's renewal billed the year: the
/// year is given back whole, -29000, and January is billed whole on the adjustment, 2900; Feb 1
/// renews a month.
#[tokio::test]
async fn an_immediate_change_from_annual_to_monthly_on_a_renewal_bills_each_month() {
    let (env, _) = usage_env().await;
    let monthly = fee_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let annual = fee_plan(&env, "Annual", DieselBillingPeriodEnum::Annual, dec!(290)).await;
    let sub_id = start(&env, annual, day(2024, 1, 1)).await;
    env.process_cycles().await;
    assert_eq!(period(&env, sub_id).await.0, day(2025, 1, 1));
    change_now(&env, sub_id, monthly, day(2025, 1, 1)).await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2025, 1, 1), Some(day(2025, 2, 1)))
    );
    env.process_cycles().await;
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2025-01-01 29000",
            "Recurring 2025-01-01: Platform Fee 2025-01-01..2026-01-01 29000",
            "note on Recurring 2025-01-01: Platform Fee 2025-01-01..2026-01-01 -29000",
            "Adjustment 2025-01-01: Platform Fee 2025-01-01..2025-02-01 2900",
            "Recurring 2025-02-01: Platform Fee 2025-02-01..2025-03-01 2900",
            "Recurring 2025-03-01: Platform Fee 2025-03-01..2025-04-01 2900",
        ]
    );
}

/// Yearly 290.00 to monthly 29.00 on Mar 1, 2024, the first day of a month on the monthly grid:
/// the year gives back its 306 days left of 366, 29000 × 306/366 = 24246, and March is billed
/// whole on the adjustment, 2900; Apr 1 renews a month.
#[tokio::test]
async fn an_immediate_change_from_annual_to_monthly_on_a_month_start_bills_each_month() {
    let (env, _) = usage_env().await;
    let monthly = fee_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let annual = fee_plan(&env, "Annual", DieselBillingPeriodEnum::Annual, dec!(290)).await;
    let sub_id = start(&env, annual, day(2024, 1, 1)).await;
    change_now(&env, sub_id, monthly, day(2024, 3, 1)).await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 3, 1), Some(day(2024, 4, 1)))
    );
    env.process_cycles().await;
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2025-01-01 29000",
            "note on Recurring 2024-01-01: Platform Fee 2024-01-01..2025-01-01 -24246",
            "Adjustment 2024-03-01: Platform Fee 2024-03-01..2024-04-01 2900",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 2900",
            "Recurring 2024-05-01: Platform Fee 2024-05-01..2024-06-01 2900",
        ]
    );
}

/// Yearly to monthly on Mar 15, 2024 (as above), back to yearly on May 10: May gives back 2900 ×
/// 22/31 = 2058; the year runs from the next renewal, Jun 1 (R-084), so the stub is 22 of the 366
/// days of [2023-06-01, 2024-06-01), 29000 × 22/366 = 1743, and Jun 1 bills the year.
#[tokio::test]
async fn a_change_back_to_annual_after_a_shorter_cadence_bills_the_year_once() {
    let (env, _) = usage_env().await;
    let monthly = fee_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let annual = fee_plan(&env, "Annual", DieselBillingPeriodEnum::Annual, dec!(290)).await;
    let sub_id = start(&env, annual, day(2024, 1, 1)).await;
    change_now(&env, sub_id, monthly, day(2024, 3, 15)).await;
    env.process_cycles().await;
    env.process_cycles().await;
    change_now(&env, sub_id, annual, day(2024, 5, 10)).await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 5, 1), Some(day(2024, 6, 1)))
    );
    env.process_cycles().await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 6, 1), Some(day(2025, 6, 1)))
    );

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2025-01-01 29000",
            "note on Recurring 2024-01-01: Platform Fee 2024-01-01..2025-01-01 -23137",
            "Adjustment 2024-03-15: Platform Fee (prorated) 2024-03-15..2024-04-01 1590",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 2900",
            "Recurring 2024-05-01: Platform Fee 2024-05-01..2024-06-01 2900",
            "note on Recurring 2024-05-01: Platform Fee 2024-05-01..2024-06-01 -2058",
            "Adjustment 2024-05-10: Platform Fee (prorated) 2024-05-10..2024-06-01 1743",
            "Recurring 2024-06-01: Platform Fee 2024-06-01..2025-06-01 29000",
        ]
    );
}

/// An amendment moving the monthly 29.00 fee onto a yearly 290.00 price on Jan 15, 2024: the
/// subscription renews yearly as if created with that fee, its grid keeping the billing start
/// (`record_cadence_change_tx`). The stub runs to that grid's year end, 29000 × 352/366 = 27891,
/// where the next renewal is: no day billed twice. January gives back 2900 × 17/31 = 1590 on a
/// note, never netted on the adjustment too: the month and the year are two charges (R-080).
#[tokio::test]
async fn an_amendment_to_a_yearly_price_bills_the_stub_to_its_grid_renewal() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    let (env, _) = usage_env().await;
    let monthly = fee_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    let platform = env.get_subscription_components(sub_id).await[0].id;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: platform,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Annual,
                            currency: "EUR".to_string(),
                            pricing: Pricing::Rate { rate: dec!(290) },
                            credit_unit: None,
                        }),
                    }],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2024, 1, 15),
        )
        .await
        .expect("an immediate amendment");
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 1, 1), Some(day(2025, 1, 1)))
    );
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "note on Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 -1590",
            "Adjustment 2024-01-15: Platform Fee (adjustment) 2024-01-15..2025-01-01 27891",
            "Recurring 2025-01-01: Platform Fee 2025-01-01..2026-01-01 29000",
        ]
    );
}

/// Amends the subscription's Platform Fee to `rate` a `cadence` on `on`, its lineage kept.
async fn amend_platform_fee(
    env: &TestEnv,
    sub_id: SubscriptionId,
    cadence: meteroid_store::domain::BillingPeriodEnum,
    rate: Decimal,
    on: NaiveDate,
) {
    amend_platform_fee_with(
        env,
        sub_id,
        cadence,
        rate,
        on,
        meteroid_store::domain::subscription_amendment::AddOnChanges::default(),
    )
    .await;
}

/// The same, with `add_on_changes`.
async fn amend_platform_fee_with(
    env: &TestEnv,
    sub_id: SubscriptionId,
    cadence: meteroid_store::domain::BillingPeriodEnum,
    rate: Decimal,
    on: NaiveDate,
    add_on_changes: meteroid_store::domain::subscription_amendment::AddOnChanges,
) {
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    let platform = env.get_subscription_components(sub_id).await[0].id;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: platform,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence,
                            currency: "EUR".to_string(),
                            pricing: Pricing::Rate { rate },
                            credit_unit: None,
                        }),
                    }],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes,
            },
            on,
        )
        .await
        .expect("an immediate amendment");
}

/// An amendment moving the yearly 120.00 fee onto 10.00 a month on Mar 15, 2024, its lineage
/// kept: the year owes 12000 less its 292 days left of 366, so its note gives back 9574; the
/// monthly run bills on its own grid, its stub to Apr 1 over March, 1000 × 17/31 = 548 (it was
/// netted into the year's note, 9026). Each renewal from Apr 1 bills its month, 1000 (R-079, R-084).
#[tokio::test]
async fn an_amendment_from_a_yearly_to_a_monthly_price_bills_each_month_at_its_price() {
    use meteroid_store::domain::BillingPeriodEnum;

    let (env, _) = usage_env().await;
    let annual = fee_plan(&env, "Annual", DieselBillingPeriodEnum::Annual, dec!(120)).await;
    let sub_id = start(&env, annual, day(2024, 1, 1)).await;
    amend_platform_fee(
        &env,
        sub_id,
        BillingPeriodEnum::Monthly,
        dec!(10),
        day(2024, 3, 15),
    )
    .await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2024, 3, 1), Some(day(2024, 4, 1)))
    );
    for _ in 0..3 {
        env.process_cycles().await;
    }

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2025-01-01 12000",
            "note on Recurring 2024-01-01: Platform Fee 2024-01-01..2025-01-01 -9574",
            "Adjustment 2024-03-15: Platform Fee (prorated) 2024-03-15..2024-04-01 548",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 1000",
            "Recurring 2024-05-01: Platform Fee 2024-05-01..2024-06-01 1000",
            "Recurring 2024-06-01: Platform Fee 2024-06-01..2024-07-01 1000",
        ]
    );
}

/// The same amendment buying a 50.00 one-time add-on: it opens the monthly grid's plan window,
/// the plan version kept, so the add-on is no plan's one-time fee and bills 5000 on the
/// amendment's adjustment invoice (R-093).
#[tokio::test]
async fn a_one_time_add_on_bought_with_a_change_of_cadence_bills() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::enums::FeeTypeEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::subscription_add_ons::{
        CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
    };
    use meteroid_store::domain::subscription_amendment::AddOnChanges;
    use meteroid_store::repositories::add_ons::AddOnInterface;

    let (env, _) = usage_env().await;
    let annual = fee_plan(&env, "Annual", DieselBillingPeriodEnum::Annual, dec!(120)).await;
    let sub_id = start(&env, annual, day(2024, 1, 1)).await;
    let setup = env
        .store()
        .create_add_on_from_ref(
            "Setup".to_string(),
            ProductRef::New {
                name: "Setup Product".to_string(),
                fee_type: FeeTypeEnum::OneTime,
                fee_structure: FeeStructure::OneTime {},
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: Pricing::OneTime {
                    unit_price: dec!(50),
                    quantity: 1,
                    model: None,
                },
                credit_unit: None,
            }),
            None,
            true,
            Some(10),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("a one-time add-on")
        .id;
    amend_platform_fee_with(
        &env,
        sub_id,
        BillingPeriodEnum::Monthly,
        dec!(10),
        day(2024, 3, 15),
        AddOnChanges {
            added: vec![CreateSubscriptionAddOn {
                add_on_id: setup,
                customization: SubscriptionAddOnCustomization::None,
                quantity: 1,
            }],
            edited: vec![],
            removed: vec![],
        },
    )
    .await;
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2025-01-01 12000",
            "note on Recurring 2024-01-01: Platform Fee 2024-01-01..2025-01-01 -9574",
            "Adjustment 2024-03-15: Platform Fee (prorated) 2024-03-15..2024-04-01 548",
            "Adjustment 2024-03-15: Setup (one-time) 2024-03-15..2024-03-15 5000",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 1000",
        ]
    );
}

/// A Platform Fee of 29.00 a month in advance and a yearly Arrears Fee of 366.00, from the billing
/// start's grid: the subscription renews monthly.
async fn monthly_with_a_yearly_arrears_fee(env: &TestEnv) -> PlanVersionId {
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Mixed", version_id)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                dec!(29),
            ),
            SeedComp::extra_recurring(
                PriceComponentId::new(),
                "Arrears Fee",
                PRODUCT_RECURRING_ARREARS_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Annual,
                dec!(366),
                1,
                BillingType::Arrears,
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    version_id
}

/// A yearly Platform Fee of 120.00 in advance and a yearly Arrears Fee of 366.00, from the billing
/// start's grid: the subscription renews yearly.
async fn yearly_with_an_arrears_fee(env: &TestEnv) -> PlanVersionId {
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Yearly mixed", version_id)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Annual,
                dec!(120),
            ),
            SeedComp::extra_recurring(
                PriceComponentId::new(),
                "Arrears Fee",
                PRODUCT_RECURRING_ARREARS_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Annual,
                dec!(366),
                1,
                BillingType::Arrears,
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    version_id
}

/// Yearly 9.99 from Jan 13, 2025, changed on Jan 21 to monthly 29.00 with a 25.00 support fee
/// (origin Feb 13); its fee amended on Jan 24 to 500.00 a year: the month gives back its days from
/// Jan 24 and the year's stub bills 50000 × 20/366 = 2732 under its year (they were netted, 861).
/// The year billed whole on Feb 13, then amended at the period's end to 49.99 a month: Mar 13
/// bills the month, 4999, and the year gives back what it billed past Mar 13, 50000 − 50000 ×
/// 28/365 = 46164.
/// Rules: R-084, R-077, R-079
#[tokio::test]
async fn an_amendment_at_the_period_end_to_a_month_bills_it_and_gives_the_year_back() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    let (env, _) = usage_env().await;
    let at = |m: u32, d: u32, h: u32| day(2025, m, d).and_hms_opt(h, 0, 0).unwrap().and_utc();
    env.set_now(at(1, 13, 9));
    let yearly = fee_plan(&env, "Yearly", DieselBillingPeriodEnum::Annual, dec!(9.99)).await;
    let monthly = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Monthly", monthly)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                dec!(29),
            ),
            SeedComp::extra_recurring(
                PriceComponentId::new(),
                "Support",
                PRODUCT_RECURRING_ARREARS_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                dec!(25),
                1,
                BillingType::Advance,
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    let sub_id = start(&env, yearly, day(2025, 1, 13)).await;
    env.set_now(at(1, 21, 9));
    change_now(&env, sub_id, monthly, day(2025, 1, 21)).await;
    env.set_now(at(1, 24, 9));
    amend_platform_fee(
        &env,
        sub_id,
        BillingPeriodEnum::Annual,
        dec!(500),
        day(2025, 1, 24),
    )
    .await;
    env.set_now(at(2, 13, 9));
    env.process_cycles().await;
    let platform = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .filter(|c| c.name == "Platform Fee" && c.effective_to.is_none())
        .max_by_key(|c| c.effective_from)
        .expect("the platform fee")
        .id;
    env.services()
        .schedule_amendment(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::EndOfPeriod,
                component_changes: ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: platform,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_string(),
                            pricing: Pricing::Rate { rate: dec!(49.99) },
                            credit_unit: None,
                        }),
                    }],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
        )
        .await
        .expect("the amendment at the period end");
    // The renewal of Feb 13 is issued when its grace ends, before the amendment takes effect.
    env.set_now(at(2, 20, 9));
    env.process_cycles().await;
    env.set_now(at(3, 13, 9));
    env.process_cycles().await;
    env.set_now(at(4, 13, 9));
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-13: Platform Fee 2025-01-13..2026-01-13 999",
            "note on Recurring 2025-01-13: Platform Fee 2025-01-13..2026-01-13 -977",
            "Adjustment 2025-01-21: Platform Fee (prorated) 2025-01-21..2025-02-13 2152",
            "note on Adjustment 2025-01-21: Platform Fee (prorated) 2025-01-21..2025-02-13 -1871",
            "Adjustment 2025-01-24: Platform Fee (adjustment) 2025-01-24..2025-02-13 2732",
            "Recurring 2025-02-13: Support 2025-01-21..2025-02-13 1855",
            "Recurring 2025-02-13: Platform Fee 2025-02-13..2026-02-13 50000",
            "note on Recurring 2025-02-13: Platform Fee 2025-02-13..2026-02-13 -46164",
            "Recurring 2025-03-13: Support 2025-02-13..2025-03-13 2500",
            "Recurring 2025-03-13: Platform Fee 2025-03-13..2025-04-13 4999",
            "Recurring 2025-04-13: Support 2025-03-13..2025-04-13 2500",
            "Recurring 2025-04-13: Platform Fee 2025-04-13..2025-05-13 4999",
        ]
    );
}

/// Yearly 120.00 in advance and 366.00 in arrears from Jan 1, 2025; on Jan 2 the arrears fee
/// is amended to 29.00 a month, so the subscription renews monthly, and it is cancelled at the
/// period's end, Feb 1. The year billed in advance gives back its unused days, 12000 × 334/365 =
/// 10981; the arrears fee bills 36600 − 36600 × 364/365 = 100 and 2900 × 30/31 = 2806.
/// Rules: R-077, R-079, R-084
#[tokio::test]
async fn a_cancellation_at_a_monthly_end_gives_back_the_rest_of_a_year_billed_in_advance() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::subscriptions::{
        CancellationEffectiveAt, SubscriptionInterfaceAuto,
    };

    let (env, _) = usage_env().await;
    // The cancellation is requested in the period it ends.
    env.set_now(day(2025, 1, 2).and_hms_opt(9, 0, 0).unwrap().and_utc());
    let yearly = yearly_with_an_arrears_fee(&env).await;
    let sub_id = start(&env, yearly, day(2025, 1, 1)).await;
    let fee = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("the subscription")
        .price_components
        .iter()
        .find(|c| c.name == "Arrears Fee")
        .expect("the arrears fee")
        .id;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: fee,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_string(),
                            pricing: Pricing::ExtraRecurring {
                                unit_price: dec!(29),
                                quantity: 1,
                                model: None,
                            },
                            credit_unit: None,
                        }),
                    }],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2025, 1, 2),
        )
        .await
        .expect("the amendment");
    assert_eq!(period(&env, sub_id).await.1, Some(day(2025, 2, 1)));
    cancel(&env, sub_id, CancellationEffectiveAt::EndOfBillingPeriod).await;
    env.set_now(day(2025, 2, 1).and_hms_opt(9, 0, 0).unwrap().and_utc());
    for _ in 0..2 {
        env.process_cycles().await;
    }

    let documents = documents(&env, sub_id).await;
    assert!(
        documents.contains(
            &"note on Recurring 2025-01-01: Platform Fee 2025-01-01..2026-01-01 -10981".to_owned()
        ),
        "{documents:#?}"
    );
    let billed: i64 = documents
        .iter()
        .map(|line| line.rsplit(' ').next().unwrap().parse::<i64>().unwrap())
        .sum();
    assert_eq!(billed, 12000 - 10981 + 100 + 2806, "{documents:#?}");
}

/// As above, changed at the period's end, Feb 1, to 9.99 a month instead of cancelled: the year
/// billed in advance gives back its unused days, 12000 × 334/365 = 10981, as the cancellation
/// does; the arrears fee bills 100 and 2806, and the new plan its month, 999.
/// Rules: R-077, R-079, R-084
#[tokio::test]
async fn a_change_at_a_monthly_end_gives_back_the_rest_of_a_year_billed_in_advance() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;

    let (env, _) = usage_env().await;
    // The change is scheduled in the period it ends.
    env.set_now(day(2025, 1, 2).and_hms_opt(9, 0, 0).unwrap().and_utc());
    let yearly = yearly_with_an_arrears_fee(&env).await;
    let sub_id = start(&env, yearly, day(2025, 1, 1)).await;
    let fee = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("the subscription")
        .price_components
        .iter()
        .find(|c| c.name == "Arrears Fee")
        .expect("the arrears fee")
        .id;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: fee,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_string(),
                            pricing: Pricing::ExtraRecurring {
                                unit_price: dec!(29),
                                quantity: 1,
                                model: None,
                            },
                            credit_unit: None,
                        }),
                    }],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2025, 1, 2),
        )
        .await
        .expect("the amendment");
    assert_eq!(period(&env, sub_id).await.1, Some(day(2025, 2, 1)));
    let monthly = fee_plan(
        &env,
        "Monthly",
        DieselBillingPeriodEnum::Monthly,
        dec!(9.99),
    )
    .await;
    switch_at_period_end(&env, sub_id, monthly).await;
    env.set_now(day(2025, 2, 1).and_hms_opt(9, 0, 0).unwrap().and_utc());
    for _ in 0..2 {
        env.process_cycles().await;
    }

    let documents = documents(&env, sub_id).await;
    assert!(
        documents.contains(
            &"note on Recurring 2025-01-01: Platform Fee 2025-01-01..2026-01-01 -10981".to_owned()
        ),
        "{documents:#?}"
    );
    let billed: i64 = documents
        .iter()
        .map(|line| line.rsplit(' ').next().unwrap().parse::<i64>().unwrap())
        .sum();
    assert_eq!(billed, 12000 - 10981 + 100 + 2806 + 999, "{documents:#?}");
}

/// As above, its invoices left drafts: January's, issued after the change, has the year it billed
/// in advance give back 12000 × 334/365 = 10981, as an invoice issued before the change would.
/// Rules: R-077, R-078
#[tokio::test]
async fn a_change_at_a_monthly_end_gives_back_a_year_billed_on_a_draft_once_it_is_issued() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;

    let (env, _) = usage_env().await;
    env.set_now(day(2025, 1, 2).and_hms_opt(9, 0, 0).unwrap().and_utc());
    let yearly = yearly_with_an_arrears_fee(&env).await;
    let sub_id = subscription()
        .plan_version(yearly)
        .start_date(day(2025, 1, 1))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    let fee = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("the subscription")
        .price_components
        .iter()
        .find(|c| c.name == "Arrears Fee")
        .expect("the arrears fee")
        .id;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: fee,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_string(),
                            pricing: Pricing::ExtraRecurring {
                                unit_price: dec!(29),
                                quantity: 1,
                                model: None,
                            },
                            credit_unit: None,
                        }),
                    }],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2025, 1, 2),
        )
        .await
        .expect("the amendment");
    let monthly = fee_plan(
        &env,
        "Monthly",
        DieselBillingPeriodEnum::Monthly,
        dec!(9.99),
    )
    .await;
    switch_at_period_end(&env, sub_id, monthly).await;
    env.set_now(day(2025, 2, 1).and_hms_opt(9, 0, 0).unwrap().and_utc());
    for _ in 0..2 {
        env.process_cycles().await;
    }
    let january = invoice_of(&env, sub_id, day(2025, 1, 1)).await;
    assert_eq!(
        january.status,
        meteroid_store::domain::enums::InvoiceStatusEnum::Draft
    );
    env.manual_finalize_invoice(january.id)
        .await
        .expect("January issued");

    let documents = documents(&env, sub_id).await;
    assert!(
        documents.contains(
            &"note on Recurring 2025-01-01: Platform Fee 2025-01-01..2026-01-01 -10981".to_owned()
        ),
        "{documents:#?}"
    );
    let billed: i64 = documents
        .iter()
        .map(|line| line.rsplit(' ').next().unwrap().parse::<i64>().unwrap())
        .sum();
    assert_eq!(billed, 12000 - 10981 + 100 + 2806 + 999, "{documents:#?}");
}

/// The yearly fee's lines over every document.
async fn yearly_fee_lines(env: &TestEnv, sub_id: SubscriptionId) -> Vec<String> {
    documents(env, sub_id)
        .await
        .into_iter()
        .filter(|line| line.contains("Arrears Fee") && !line.ends_with(" 0"))
        .collect()
}

/// Changed to quarterly 90.00 on Apr 1 after that day's invoice, which billed nothing of the year
/// (Apr 1 is no yearly boundary): the year owes Jan 1 to Apr 1, 36600 × 91/366 = 9100, on the
/// change's adjustment that day (R-051), once.
#[tokio::test]
async fn a_yearly_fee_in_arrears_ended_on_a_renewal_day_by_a_change_of_cadence_settles_once() {
    let (env, _) = usage_env().await;
    let version_id = monthly_with_a_yearly_arrears_fee(&env).await;
    let quarterly = fee_plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(90),
    )
    .await;
    let sub_id = start(&env, version_id, day(2024, 1, 1)).await;
    for _ in 0..3 {
        env.process_cycles().await;
    }
    invoice_of(&env, sub_id, day(2024, 4, 1)).await;
    change_now(&env, sub_id, quarterly, day(2024, 4, 1)).await;
    for _ in 0..6 {
        env.process_cycles().await;
    }
    assert_eq!(
        yearly_fee_lines(&env, sub_id).await,
        vec!["Adjustment 2024-04-01: Arrears Fee 2024-01-01..2024-04-01 9100"]
    );
}

/// Monthly 29.00 with bandwidth at 0.10 a unit billed yearly, from Jan 1, 2024, 500 units used by
/// Apr 1; changed that day, after its invoice, to a plan of a fee alone: the year's usage ends
/// there and bills 5000 on the change's adjustment, once (R-011).
#[tokio::test]
async fn usage_billed_yearly_ended_on_a_renewal_day_by_a_change_bills_on_its_adjustment() {
    let (env, usage) = usage_env().await;
    set_usage(&usage, day(2024, 1, 1), day(2024, 4, 1), 500);
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Usage yearly", version_id)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                dec!(29),
            ),
            SeedComp::usage(
                PriceComponentId::new(),
                "Bandwidth",
                PRODUCT_BANDWIDTH_ID,
                METRIC_BANDWIDTH,
                PriceId::new(),
                DieselBillingPeriodEnum::Annual,
                UsagePricingModel::PerUnit { rate: dec!(0.10) },
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    let monthly = fee_plan(
        &env,
        "Monthly",
        DieselBillingPeriodEnum::Monthly,
        dec!(9.99),
    )
    .await;
    let sub_id = start(&env, version_id, day(2024, 1, 1)).await;
    for _ in 0..3 {
        env.process_cycles().await;
    }
    invoice_of(&env, sub_id, day(2024, 4, 1)).await;
    change_now(&env, sub_id, monthly, day(2024, 4, 1)).await;
    for _ in 0..6 {
        env.process_cycles().await;
    }
    let bandwidth: Vec<String> = documents(&env, sub_id)
        .await
        .into_iter()
        .filter(|line| line.contains("Bandwidth") && !line.ends_with(" 0"))
        .collect();
    assert_eq!(
        bandwidth,
        vec!["Adjustment 2024-04-01: Bandwidth 2024-01-01..2024-04-01 5000"]
    );
}

/// The same switch scheduled for the period's end, applied before the Apr 1 invoice is written:
/// that invoice settles the year to Apr 1, 36600 × 91/366 = 9100, once (R-011).
#[tokio::test]
async fn a_yearly_fee_in_arrears_ended_by_a_scheduled_change_of_cadence_settles_once() {
    let (env, _) = usage_env().await;
    let version_id = monthly_with_a_yearly_arrears_fee(&env).await;
    let quarterly = fee_plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(90),
    )
    .await;
    let sub_id = start(&env, version_id, day(2024, 1, 1)).await;
    env.process_cycles().await;
    env.process_cycles().await;
    switch_at_period_end(&env, sub_id, quarterly).await;
    for _ in 0..6 {
        env.process_cycles().await;
    }
    assert_eq!(
        yearly_fee_lines(&env, sub_id).await,
        vec!["Recurring 2024-04-01: Arrears Fee 2024-01-01..2024-04-01 9100"]
    );
}

/// Changed on Mar 15 to a plan of the monthly fee alone, the cadence kept: the year owes Jan 1 to
/// Mar 15, 36600 × 74/366 = 7400, on the next renewal, Apr 1 (R-011), never again on Jan 1, 2025.
#[tokio::test]
async fn a_yearly_fee_in_arrears_ended_mid_year_settles_on_the_next_renewal() {
    let (env, _) = usage_env().await;
    let version_id = monthly_with_a_yearly_arrears_fee(&env).await;
    let monthly = fee_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let sub_id = start(&env, version_id, day(2024, 1, 1)).await;
    env.process_cycles().await;
    env.process_cycles().await;
    change_now(&env, sub_id, monthly, day(2024, 3, 15)).await;
    for _ in 0..11 {
        env.process_cycles().await;
    }
    assert_eq!(period(&env, sub_id).await.0, day(2025, 2, 1));
    assert_eq!(
        yearly_fee_lines(&env, sub_id).await,
        vec!["Recurring 2024-04-01: Arrears Fee 2024-01-01..2024-03-15 7400"]
    );
}

/// Monthly 10.00 with a yearly 120.00 add-on bought Jan 1, 2024, changed to quarterly 30.00 on
/// Mar 15 (origin Apr 1). The add-on keeps its year: nothing billed or given back at the change,
/// and it renews Jan 1, 2025. March gives back 1000 × 17/31 = 548; the quarterly stub is 17 days
/// of [Jan 1, Apr 1), 3000 × 17/91 = 560, then a quarter each renewal.
/// Rules: R-084
#[tokio::test]
async fn an_add_on_kept_through_a_change_of_cadence_keeps_its_year() {
    use meteroid_store::domain::plan_version_add_ons::PlanVersionAddOnNew;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::subscription_add_ons::{
        CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
    };
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::domain::{BillingPeriodEnum, enums::FeeTypeEnum};
    use meteroid_store::repositories::add_ons::AddOnInterface;
    use meteroid_store::repositories::plan_version_add_ons::PlanVersionAddOnInterface;

    let (env, _) = usage_env().await;
    let monthly = fee_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(10)).await;
    let quarterly = fee_plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        dec!(30),
    )
    .await;
    let add_on = env
        .store()
        .create_add_on_from_ref(
            "Archive".to_string(),
            ProductRef::New {
                name: "Archive Product".to_string(),
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Annual,
                currency: "EUR".to_string(),
                pricing: Pricing::Rate { rate: dec!(120) },
                credit_unit: None,
            }),
            None,
            true,
            Some(1),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("a yearly add-on")
        .id;
    env.store()
        .attach_add_on_to_plan_version(PlanVersionAddOnNew {
            plan_version_id: quarterly,
            add_on_id: add_on,
            price_id: None,
            self_serviceable: None,
            max_instances_per_subscription: None,
            tenant_id: TENANT_ID,
        })
        .await
        .expect("the quarterly plan offers the add-on");
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![CreateSubscriptionAddOn {
                        add_on_id: add_on,
                        customization: SubscriptionAddOnCustomization::None,
                        quantity: 1,
                    }],
                    edited: vec![],
                    removed: vec![],
                },
            },
            day(2024, 1, 1),
        )
        .await
        .expect("add the add-on");
    env.process_cycles().await;
    env.process_cycles().await;
    change_now(&env, sub_id, quarterly, day(2024, 3, 15)).await;
    for _ in 0..4 {
        env.process_cycles().await;
    }

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 1000",
            "Adjustment 2024-01-01: Archive 2024-01-01..2025-01-01 12000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 1000",
            "Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 1000",
            "note on Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 -548",
            "Adjustment 2024-03-15: Platform Fee (prorated) 2024-03-15..2024-04-01 560",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-07-01 3000",
            "Recurring 2024-07-01: Platform Fee 2024-07-01..2024-10-01 3000",
            "Recurring 2024-10-01: Platform Fee 2024-10-01..2025-01-01 3000",
            "Recurring 2025-01-01: Platform Fee 2025-01-01..2025-04-01 3000",
            "Recurring 2025-01-01: Archive 2025-01-01..2026-01-01 12000",
        ]
    );
}

/// A monthly 10.00 plan, a yearly 120.00 one, and a yearly 120.00 add-on both offer.
async fn plans_with_a_yearly_add_on(env: &TestEnv) -> (PlanVersionId, PlanVersionId, AddOnId) {
    use meteroid_store::domain::plan_version_add_ons::PlanVersionAddOnNew;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::{BillingPeriodEnum, enums::FeeTypeEnum};
    use meteroid_store::repositories::add_ons::AddOnInterface;
    use meteroid_store::repositories::plan_version_add_ons::PlanVersionAddOnInterface;

    let monthly = fee_plan(env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(10)).await;
    let yearly = fee_plan(env, "Yearly", DieselBillingPeriodEnum::Annual, dec!(120)).await;
    let add_on = env
        .store()
        .create_add_on_from_ref(
            "Archive".to_string(),
            ProductRef::New {
                name: "Archive Product".to_string(),
                fee_type: FeeTypeEnum::Rate,
                fee_structure: FeeStructure::Rate {},
            },
            PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Annual,
                currency: "EUR".to_string(),
                pricing: Pricing::Rate { rate: dec!(120) },
                credit_unit: None,
            }),
            None,
            true,
            Some(1),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("a yearly add-on")
        .id;
    for plan_version_id in [monthly, yearly] {
        env.store()
            .attach_add_on_to_plan_version(PlanVersionAddOnNew {
                plan_version_id,
                add_on_id: add_on,
                price_id: None,
                self_serviceable: None,
                max_instances_per_subscription: None,
                tenant_id: TENANT_ID,
            })
            .await
            .expect("the plan offers the add-on");
    }
    (monthly, yearly, add_on)
}

async fn buy_add_on(env: &TestEnv, sub_id: SubscriptionId, add_on: AddOnId, on: NaiveDate) {
    use meteroid_store::domain::subscription_add_ons::{
        CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
    };
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![CreateSubscriptionAddOn {
                        add_on_id: add_on,
                        customization: SubscriptionAddOnCustomization::None,
                        quantity: 1,
                    }],
                    edited: vec![],
                    removed: vec![],
                },
            },
            on,
        )
        .await
        .expect("add the add-on");
}

/// Yearly 120.00 from Jan 1, 2024, monthly 10.00 from Jan 1, 2025 and a yearly 120.00 add-on
/// bought Feb 10 on that grid's year to Jan 1, 2026: 12000 × 325/365 = 10685. Back to yearly at
/// Mar 1, the add-on keeps its year; cancelled at the period's end, Mar 1, 2026 bills the
/// add-on's year begun Jan 1 up to its end, 12000 − 12000 × 306/365 = 1940.
/// Rules: R-084, R-002
#[tokio::test]
async fn an_add_on_kept_through_a_change_of_cadence_bills_its_last_year_up_to_the_end() {
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let (env, _) = usage_env().await;
    // The cancellation is requested in the period it ends.
    env.set_now(day(2025, 3, 1).and_hms_opt(9, 0, 0).unwrap().and_utc());
    let (monthly, yearly, add_on) = plans_with_a_yearly_add_on(&env).await;
    let sub_id = start(&env, yearly, day(2024, 1, 1)).await;
    switch_at_period_end(&env, sub_id, monthly).await;
    for _ in 0..2 {
        env.process_cycles().await;
    }
    buy_add_on(&env, sub_id, add_on, day(2025, 2, 10)).await;
    switch_at_period_end(&env, sub_id, yearly).await;
    env.process_cycles().await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2025, 3, 1), Some(day(2026, 3, 1)))
    );
    cancel(&env, sub_id, CancellationEffectiveAt::EndOfBillingPeriod).await;
    env.set_now(day(2026, 3, 1).and_hms_opt(9, 0, 0).unwrap().and_utc());
    for _ in 0..2 {
        env.process_cycles().await;
    }

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2025-01-01 12000",
            "Recurring 2025-01-01: Platform Fee 2025-01-01..2025-02-01 1000",
            "Recurring 2025-02-01: Platform Fee 2025-02-01..2025-03-01 1000",
            "Adjustment 2025-02-10: Archive (prorated) 2025-02-10..2026-01-01 10685",
            "Recurring 2025-03-01: Platform Fee 2025-03-01..2026-03-01 12000",
            "Recurring 2026-03-01: Archive 2026-01-01..2026-03-01 1940",
        ]
    );
}

/// The same add-on bought on the billing start's grid: monthly from Jan 1, 2024, bought Feb 10
/// to Jan 1, 2025, 12000 × 326/366 = 10689; yearly from Mar 1, the year begun Jan 1, 2025 bills
/// up to the end on Mar 1, 2025, 12000 − 12000 × 306/365 = 1940.
/// Rules: R-084, R-002
#[tokio::test]
async fn an_add_on_kept_from_the_billing_start_grid_bills_its_last_year_up_to_the_end() {
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let (env, _) = usage_env().await;
    // The cancellation is requested in the period it ends.
    env.set_now(day(2024, 3, 1).and_hms_opt(9, 0, 0).unwrap().and_utc());
    let (monthly, yearly, add_on) = plans_with_a_yearly_add_on(&env).await;
    let sub_id = start(&env, monthly, day(2024, 1, 1)).await;
    env.process_cycles().await;
    buy_add_on(&env, sub_id, add_on, day(2024, 2, 10)).await;
    switch_at_period_end(&env, sub_id, yearly).await;
    env.process_cycles().await;
    cancel(&env, sub_id, CancellationEffectiveAt::EndOfBillingPeriod).await;
    env.set_now(day(2025, 3, 1).and_hms_opt(9, 0, 0).unwrap().and_utc());
    for _ in 0..2 {
        env.process_cycles().await;
    }

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 1000",
            "Adjustment 2024-02-10: Archive (prorated) 2024-02-10..2025-01-01 10689",
            "Recurring 2024-03-01: Platform Fee 2024-03-01..2025-03-01 12000",
            "Recurring 2025-03-01: Archive 2025-01-01..2025-03-01 1940",
        ]
    );
}

/// 123.45 a month in arrears from Jan 10, 2025, amended to 9.99 on Jan 17 and cancelled on Jan
/// 21: the final invoice bills each row its part, what is left of its price at its start less
/// at its end (R-079): 12345 − 12345 × 24/31 = 2788, then 999 × 24/31 − 999 × 20/31 = 128.
/// Rules: R-079, R-002
#[tokio::test]
async fn an_arrears_fee_amended_then_cancelled_bills_each_row_its_tiled_part() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::subscriptions::{
        CancellationEffectiveAt, SubscriptionInterfaceAuto,
    };

    let (env, _) = usage_env().await;
    // The cancellation is requested on the day it ends from.
    env.set_now(day(2025, 1, 21).and_hms_opt(9, 0, 0).unwrap().and_utc());
    let monthly = arrears_plan(
        &env,
        "Monthly",
        DieselBillingPeriodEnum::Monthly,
        dec!(123.45),
    )
    .await;
    let sub_id = start(&env, monthly, day(2025, 1, 10)).await;
    let fee = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("the subscription")
        .price_components
        .iter()
        .find(|c| c.name == "Arrears Fee")
        .expect("the arrears fee")
        .id;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: fee,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_string(),
                            pricing: Pricing::ExtraRecurring {
                                unit_price: dec!(9.99),
                                quantity: 1,
                                model: None,
                            },
                            credit_unit: None,
                        }),
                    }],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2025, 1, 17),
        )
        .await
        .expect("the amendment");
    cancel(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(day(2025, 1, 21)),
    )
    .await;
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-21: Arrears Fee (2025-01-17 - 2025-01-21) (2025-01-17 - 2025-01-21) 2025-01-17..2025-01-21 128",
            "Recurring 2025-01-21: Arrears Fee (2025-01-10 - 2025-01-17) (2025-01-10 - 2025-01-17) 2025-01-10..2025-01-17 2788",
        ]
    );
}

/// 9.99 a month (A) from Jan 9, 2025, changed to B at 9.99 on Feb 12 (892 for its 25 of 28 days,
/// A giving back 892), B amended to 49.99 on Feb 16 (3000 more), cancelled on Feb 21 with usage
/// [Feb 12, Feb 21) on the final invoice. The cancellation gives back what is left past Feb 21:
/// 571 of the stub and 2286 of the amendment's line. Issuing the final invoice settles the plan
/// change again, which owes nothing more: no note takes the stub's first days back.
/// Rules: R-080, R-077
#[tokio::test]
async fn a_plan_change_settled_again_takes_back_nothing_later_changes_settled() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::subscriptions::{
        CancellationEffectiveAt, SubscriptionInterfaceAuto,
    };
    let (env, usage) = usage_env().await;
    // The cancellation is requested on the day it ends from.
    env.set_now(day(2025, 2, 21).and_hms_opt(9, 0, 0).unwrap().and_utc());
    set_usage(&usage, day(2025, 2, 12), day(2025, 2, 21), 22);
    let a = plan(&env, "A", DieselBillingPeriodEnum::Monthly, dec!(9.99)).await;
    let b = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "B", b)
        .components(vec![
            SeedComp::extra_recurring(
                PriceComponentId::new(),
                "B Fee",
                PRODUCT_EXTRA_ADVANCE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                dec!(9.99),
                1,
                BillingType::Advance,
            ),
            SeedComp::usage(
                PriceComponentId::new(),
                "B units",
                PRODUCT_BANDWIDTH_ID,
                METRIC_BANDWIDTH,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit { rate: dec!(0.07) },
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .expect("plan B");
    let sub_id = start(&env, a, day(2025, 1, 9)).await;
    env.process_cycles().await;
    change_now(&env, sub_id, b, day(2025, 2, 12)).await;
    let fee = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("the subscription")
        .price_components
        .iter()
        .filter(|c| c.effective_to.is_none())
        .find(|c| c.name == "B Fee")
        .expect("B's fee")
        .id;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: fee,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_string(),
                            pricing: Pricing::ExtraRecurring {
                                unit_price: dec!(49.99),
                                quantity: 1,
                                model: None,
                            },
                            credit_unit: None,
                        }),
                    }],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2025, 2, 16),
        )
        .await
        .expect("the amendment");
    cancel(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(day(2025, 2, 21)),
    )
    .await;
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-09: Platform Fee 2025-01-09..2025-02-09 999",
            "Recurring 2025-02-09: Platform Fee 2025-02-09..2025-03-09 999",
            "note on Recurring 2025-02-09: Platform Fee 2025-02-09..2025-03-09 -892",
            "Adjustment 2025-02-12: B Fee (prorated) 2025-02-12..2025-03-09 892",
            "note on Adjustment 2025-02-12: B Fee (prorated) 2025-02-12..2025-03-09 -571",
            "Adjustment 2025-02-16: B Fee (adjustment) 2025-02-16..2025-03-09 3000",
            "note on Adjustment 2025-02-16: B Fee (adjustment) 2025-02-16..2025-03-09 -2286",
            "Recurring 2025-02-21: B units 2025-02-12..2025-02-21 154",
        ]
    );
}

/// 49.99 a year from Jan 23, 2025, amended to 9.99 on Jan 27 and to 500.00 the same day: the
/// first gives back 4999 × 361/365 − 999 × 361/365 = 4944 − 988 = 3956; the second owes the year
/// 4999 − 4944 + 50000 × 361/365 = 55 + 49452 = 49507, so its adjustment bills 49507 − 1043 =
/// 48464, the 3956 given back included.
/// Rules: R-079, R-080
#[tokio::test]
async fn a_second_amendment_the_same_day_bills_back_what_the_first_gave_back() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
    let (env, _) = usage_env().await;
    let annual = fee_plan(&env, "Annual", DieselBillingPeriodEnum::Annual, dec!(49.99)).await;
    let sub_id = start(&env, annual, day(2025, 1, 23)).await;
    for rate in [dec!(9.99), dec!(500)] {
        let fee = env
            .store()
            .get_subscription_details(TENANT_ID, sub_id)
            .await
            .expect("the subscription")
            .price_components
            .iter()
            .filter(|c| c.effective_to.is_none())
            .find(|c| c.name == "Platform Fee")
            .expect("the fee")
            .id;
        env.services()
            .apply_amendment_immediate_at(
                Actor::System,
                sub_id,
                TENANT_ID,
                SubscriptionAmendment {
                    apply_mode: PlanChangeMode::Immediate,
                    component_changes: ComponentChanges {
                        edited: vec![EditComponent {
                            subscription_component_id: fee,
                            name: None,
                            price_entry: PriceEntry::New(PriceInput {
                                cadence: BillingPeriodEnum::Annual,
                                currency: "EUR".to_string(),
                                pricing: Pricing::Rate { rate },
                                credit_unit: None,
                            }),
                        }],
                        added: vec![],
                        removed: vec![],
                    },
                    add_on_changes: AddOnChanges::default(),
                },
                day(2025, 1, 27),
            )
            .await
            .expect("the amendment");
    }

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-23: Platform Fee 2025-01-23..2026-01-23 4999",
            "note on Recurring 2025-01-23: Platform Fee 2025-01-23..2026-01-23 -3956",
            "Adjustment 2025-01-27: Platform Fee (adjustment) 2025-01-27..2026-01-23 48464",
        ]
    );
}

/// 49.99 a month in arrears from Jan 11, 2025, its invoices left drafts; a 15.00 one-time fee
/// added on Feb 11, a renewal day after its draft was written, bills 1500 on an adjustment left a
/// draft, which the operator issues and voids. Finalizing the renewal's draft bills January's 4999:
/// the void forgave the fee, so the facts owe nothing more of it, and none is reported unbilled.
/// Rules: R-096, R-166
#[tokio::test]
async fn a_voided_one_time_fee_is_owed_no_more_when_its_renewal_is_finalized() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::enums::{FeeTypeEnum, InvoiceStatusEnum, InvoiceType};
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::InvoiceInterface;
    use meteroid_store::services::{InvoiceDelivery, IssueAt};

    let (env, _) = usage_env().await;
    let monthly = arrears_plan(
        &env,
        "Monthly",
        DieselBillingPeriodEnum::Monthly,
        dec!(49.99),
    )
    .await;
    let sub_id = subscription()
        .plan_version(monthly)
        .start_date(day(2025, 1, 11))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    env.process_cycles().await;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![AddExtraComponent {
                        name: "Setup".to_string(),
                        product: ProductRef::New {
                            name: "Setup Product".to_string(),
                            fee_type: FeeTypeEnum::OneTime,
                            fee_structure: FeeStructure::OneTime {},
                        },
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_owned(),
                            pricing: Pricing::OneTime {
                                unit_price: dec!(15),
                                quantity: 1,
                                model: None,
                            },
                            credit_unit: None,
                        }),
                        fee_structure: None,
                    }],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2025, 2, 11),
        )
        .await
        .expect("the one-time fee");
    let at = |hour| {
        day(2025, 2, 15)
            .and_hms_opt(hour, 0, 0)
            .expect("an instant")
            .and_utc()
    };
    let invoices = env.get_invoices(sub_id).await;
    let adjustment = invoices
        .iter()
        .find(|invoice| invoice.invoice_type == InvoiceType::Adjustment)
        .expect("the adjustment");
    assert_eq!(adjustment.status, InvoiceStatusEnum::Draft, "R-166");
    env.services()
        .finalize_invoice_at(
            Actor::System,
            adjustment.id,
            TENANT_ID,
            IssueAt::At(
                day(2025, 2, 11)
                    .and_hms_opt(16, 0, 0)
                    .expect("an instant")
                    .and_utc(),
            ),
            InvoiceDelivery::Suppressed,
        )
        .await
        .expect("the adjustment issued");
    env.store()
        .void_invoice_at(Actor::System, adjustment.id, TENANT_ID, at(17))
        .await
        .expect("the void");
    let draft = invoices
        .iter()
        .find(|invoice| invoice.invoice_type == InvoiceType::Recurring)
        .expect("the renewal's draft");
    env.services()
        .finalize_invoice_at(
            Actor::System,
            draft.id,
            TENANT_ID,
            IssueAt::At(at(18)),
            InvoiceDelivery::Suppressed,
        )
        .await
        .expect("the renewal");

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-02-11: Arrears Fee 2025-01-11..2025-02-11 4999",
            "Adjustment 2025-02-11: Setup (one-time) 2025-02-11..2025-02-11 1500",
            "note on Adjustment 2025-02-11: Setup (one-time) 2025-02-11..2025-02-11 -1500",
        ]
    );
}

/// 49.99 a month from Jan 18, 2025, its invoices left drafts, with a 15.00 one-time fee added on
/// Feb 21 and the plan changed on Feb 26 (20 of February's 28 days left). The renewal's draft,
/// finalized after, is credited what the change gives back of it: 4999 × 20/28 = 3571; the
/// one-time fee the change ends reaches back over no period of its own.
/// Rules: R-077, R-080
#[tokio::test]
async fn a_draft_finalized_after_a_change_ending_a_one_time_fee_is_credited() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::enums::{FeeTypeEnum, InvoiceStatusEnum};
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::services::{InvoiceDelivery, IssueAt};

    let (env, _) = usage_env().await;
    let a = fee_plan(&env, "A", DieselBillingPeriodEnum::Monthly, dec!(49.99)).await;
    let b = fee_plan(&env, "B", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let sub_id = subscription()
        .plan_version(a)
        .start_date(day(2025, 1, 18))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    env.process_cycles().await;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![AddExtraComponent {
                        name: "Setup".to_string(),
                        product: ProductRef::New {
                            name: "Setup Product".to_string(),
                            fee_type: FeeTypeEnum::OneTime,
                            fee_structure: FeeStructure::OneTime {},
                        },
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_owned(),
                            pricing: Pricing::OneTime {
                                unit_price: dec!(15),
                                quantity: 1,
                                model: None,
                            },
                            credit_unit: None,
                        }),
                        fee_structure: None,
                    }],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2025, 2, 21),
        )
        .await
        .expect("the one-time fee");
    change_now(&env, sub_id, b, day(2025, 2, 26)).await;
    let draft = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| {
            invoice.status == InvoiceStatusEnum::Draft && invoice.invoice_date == day(2025, 2, 18)
        })
        .expect("February's draft");
    env.services()
        .finalize_invoice_at(
            Actor::System,
            draft.id,
            TENANT_ID,
            IssueAt::At(
                day(2025, 2, 27)
                    .and_hms_opt(9, 0, 0)
                    .expect("an instant")
                    .and_utc(),
            ),
            InvoiceDelivery::Suppressed,
        )
        .await
        .expect("February's invoice");

    let february: Vec<String> = documents(&env, sub_id)
        .await
        .into_iter()
        .filter(|line| line.contains("Recurring 2025-02-18"))
        .collect();
    assert_eq!(
        february,
        vec![
            "Recurring 2025-02-18: Platform Fee 2025-02-18..2025-03-18 4999",
            "note on Recurring 2025-02-18: Platform Fee 2025-02-18..2025-03-18 -3571",
        ]
    );
}

/// 49.99 a year (A) from Jan 27, 2025, switched to B at 29.00 a year at its end and paid; changed
/// to A on Feb 1, 2026 (B gives back 2900 × 360/365 = 2860 to the balance, which pays part of A's
/// 4999 × 360/365 = 4931), then back to B on Feb 4. A's line gives back 4999 × 357/365 = 4889:
/// its 2071 still due cancelled, the rest credited to the balance; the change goes through.
/// Rules: R-077, I17
#[tokio::test]
async fn a_change_crediting_an_invoice_credit_partly_paid_cancels_its_debt_and_credits_the_rest() {
    use meteroid_store::domain::enums::InvoiceStatusEnum;
    use meteroid_store::repositories::CreditNoteInterface;
    use meteroid_store::services::InvoiceDelivery;

    let (env, _) = usage_env().await;
    let a = fee_plan(&env, "A", DieselBillingPeriodEnum::Annual, dec!(49.99)).await;
    let b = fee_plan(&env, "B", DieselBillingPeriodEnum::Annual, dec!(29)).await;
    let sub_id = start(&env, a, day(2025, 1, 27)).await;
    switch_at_period_end(&env, sub_id, b).await;
    env.process_cycles().await;
    let renewal = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| {
            invoice.status == InvoiceStatusEnum::Finalized
                && invoice.billing_period_start == day(2026, 1, 27)
        })
        .expect("the renewal");
    env.services()
        .settle_invoice_payment_at(
            Actor::System,
            TENANT_ID,
            renewal.id,
            day(2026, 1, 27)
                .and_hms_opt(18, 0, 0)
                .expect("an instant")
                .and_utc(),
            Some("paid".to_string()),
            InvoiceDelivery::Suppressed,
        )
        .await
        .expect("the payment");
    change_now(&env, sub_id, a, day(2026, 2, 1)).await;
    change_now(&env, sub_id, b, day(2026, 2, 4)).await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-27: Platform Fee 2025-01-27..2026-01-27 4999",
            "Recurring 2026-01-27: Platform Fee 2026-01-27..2027-01-27 2900",
            "note on Recurring 2026-01-27: Platform Fee 2026-01-27..2027-01-27 -2860",
            "Adjustment 2026-02-01: Platform Fee (prorated) 2026-02-01..2027-01-27 4931",
            "note on Adjustment 2026-02-01: Platform Fee (prorated) 2026-02-01..2027-01-27 -4889",
            "Adjustment 2026-02-04: Platform Fee (prorated) 2026-02-04..2027-01-27 2836",
        ]
    );
    let a_line = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| invoice.billing_period_start == day(2026, 2, 1))
        .expect("A's adjustment");
    let notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, a_line.id)
        .await
        .expect("its notes");
    assert_eq!(
        notes
            .iter()
            .map(|note| (note.debt_cancelled_amount_cents, note.credited_amount_cents))
            .collect::<Vec<_>>(),
        vec![(2_071, 2_818)]
    );
}

/// 29.00 a month from Jan 6, 2025, renewed Feb 6, its fee amended on Feb 7 to 9.99 a year: no
/// change of the subscription's cadence, the year runs on the grid it was created on. February
/// gives back 2900 × 27/28 = 2796; the year bills 999 × 333/365 = 911 to Jan 6, 2026, then 999.
/// Rules: R-084, R-078
#[tokio::test]
async fn an_amendment_to_a_yearly_fee_renews_on_the_subscriptions_anniversary() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;

    let (env, _) = usage_env().await;
    let monthly = fee_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let sub_id = start(&env, monthly, day(2025, 1, 6)).await;
    env.process_cycles().await;
    let fee = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("the subscription")
        .price_components
        .iter()
        .find(|c| c.name == "Platform Fee")
        .expect("the fee")
        .id;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: fee,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Annual,
                            currency: "EUR".to_string(),
                            pricing: Pricing::Rate { rate: dec!(9.99) },
                            credit_unit: None,
                        }),
                    }],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2025, 2, 7),
        )
        .await
        .expect("the amendment");
    assert_eq!(period(&env, sub_id).await.1, Some(day(2026, 1, 6)));
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-06: Platform Fee 2025-01-06..2025-02-06 2900",
            "Recurring 2025-02-06: Platform Fee 2025-02-06..2025-03-06 2900",
            "note on Recurring 2025-02-06: Platform Fee 2025-02-06..2025-03-06 -2796",
            "Adjustment 2025-02-07: Platform Fee (adjustment) 2025-02-07..2026-01-06 911",
            "Recurring 2026-01-06: Platform Fee 2026-01-06..2027-01-06 999",
        ]
    );
}

/// 123.45 a month in arrears from Jan 20, 2025, amended on Feb 25 to 9.99 a year in arrears.
/// January's line bills January's month, never the year it falls in: Jan 20, 2026 bills the
/// month's 5 days, 12345 − 12345 × 23/28 = 2204, and the year's 329 days, 999 × 329/365 = 900.
/// Rules: R-080, R-084, R-011
#[tokio::test]
async fn a_months_line_never_bills_the_year_an_amendment_puts_it_in() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;

    let (env, _) = usage_env().await;
    let monthly = arrears_plan(
        &env,
        "Monthly",
        DieselBillingPeriodEnum::Monthly,
        dec!(123.45),
    )
    .await;
    let sub_id = start(&env, monthly, day(2025, 1, 20)).await;
    env.process_cycles().await;
    let components = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("the subscription")
        .price_components;
    let id_of = |name: &str| {
        components
            .iter()
            .find(|c| c.name == name)
            .expect("a component")
            .id
    };
    let yearly = |id, pricing| EditComponent {
        subscription_component_id: id,
        name: None,
        price_entry: PriceEntry::New(PriceInput {
            cadence: BillingPeriodEnum::Annual,
            currency: "EUR".to_string(),
            pricing,
            credit_unit: None,
        }),
    };
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![
                        yearly(
                            id_of("Arrears Fee"),
                            Pricing::ExtraRecurring {
                                unit_price: dec!(9.99),
                                quantity: 1,
                                model: None,
                            },
                        ),
                        yearly(
                            id_of("Bandwidth"),
                            Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(0.10) }),
                        ),
                    ],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2025, 2, 25),
        )
        .await
        .expect("the amendment");
    while period(&env, sub_id).await.0 < day(2026, 1, 20) {
        env.process_cycles().await;
    }

    let renewal: Vec<String> = documents(&env, sub_id)
        .await
        .into_iter()
        .filter(|line| line.starts_with("Recurring 2026-01-20") && line.contains("Fee"))
        .collect();
    assert_eq!(renewal.len(), 2, "{renewal:?}");
    assert!(
        renewal
            .iter()
            .any(|line| line.ends_with("2025-02-20..2025-02-25 2204")),
        "{renewal:?}"
    );
    assert!(
        renewal
            .iter()
            .any(|line| line.ends_with("2025-02-25..2026-01-20 900")),
        "{renewal:?}"
    );
}

/// 29.00 a month from Jan 6, 2025, amended on Feb 11 to 29.00 a year (2900 × 329/365 = 2614,
/// February giving back 2900 × 23/28 = 2382), then changed to B at 9.99 a month on Feb 14. The
/// year's 3 days now fall in February's period: the year's line gives back what it billed past
/// Feb 14, 2614 × 326/329 = 2590, and keeps 24; B bills 999 × 20/28 = 714.
/// Rules: R-080, R-077, R-084
#[tokio::test]
async fn a_line_whose_period_a_change_folds_into_another_gives_back_its_unused_time() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
    let (env, _) = usage_env().await;
    let a = plan(&env, "A", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let b = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "B", b)
        .components(vec![SeedComp::extra_recurring(
            PriceComponentId::new(),
            "B Fee",
            PRODUCT_EXTRA_ADVANCE_ID,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            dec!(9.99),
            1,
            BillingType::Advance,
        )])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    let sub_id = start(&env, a, day(2025, 1, 6)).await;
    env.process_cycles().await;
    let components = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .unwrap()
        .price_components;
    let id_of = |name: &str| components.iter().find(|c| c.name == name).unwrap().id;
    let yearly = |id, pricing| EditComponent {
        subscription_component_id: id,
        name: None,
        price_entry: PriceEntry::New(PriceInput {
            cadence: BillingPeriodEnum::Annual,
            currency: "EUR".to_string(),
            pricing,
            credit_unit: None,
        }),
    };
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![
                        yearly(id_of("Platform Fee"), Pricing::Rate { rate: dec!(29) }),
                        yearly(
                            id_of("Bandwidth"),
                            Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(0.10) }),
                        ),
                    ],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2025, 2, 11),
        )
        .await
        .expect("amend");
    change_now(&env, sub_id, b, day(2025, 2, 14)).await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-06: Platform Fee 2025-01-06..2025-02-06 2900",
            "Recurring 2025-02-06: Platform Fee 2025-02-06..2025-03-06 2900",
            "note on Recurring 2025-02-06: Platform Fee 2025-02-06..2025-03-06 -2382",
            "Adjustment 2025-02-11: Platform Fee (adjustment) 2025-02-11..2026-01-06 2614",
            "note on Adjustment 2025-02-11: Platform Fee (adjustment) 2025-02-11..2026-01-06 -2590",
            "Adjustment 2025-02-14: B Fee (prorated) 2025-02-14..2025-03-06 714",
        ]
    );
}

/// 29.00 a month from Jan 6, 2025 with bandwidth, both amended on Feb 11 to a year (the fee
/// 2900 × 329/365 = 2614), the subscription cancelled on Feb 15 with usage on its final invoice.
/// The year's 4 days fall in February's period now; the year's line keeps their worth,
/// 2614 × 4/329 = 32, and gives back 2582, once, however many changes read it again.
/// Rules: R-080, R-077, R-084
#[tokio::test]
async fn a_folded_line_keeps_its_billed_days_whichever_change_reads_it() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::subscriptions::{
        CancellationEffectiveAt, SubscriptionInterfaceAuto,
    };

    let (env, usage) = usage_env().await;
    set_usage(&usage, day(2025, 2, 6), day(2025, 2, 11), 10);
    set_usage(&usage, day(2025, 2, 11), day(2025, 2, 15), 20);
    let a = plan(&env, "A", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let sub_id = start(&env, a, day(2025, 1, 6)).await;
    env.process_cycles().await;
    let components = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("the subscription")
        .price_components;
    let id_of = |name: &str| {
        components
            .iter()
            .find(|c| c.name == name)
            .expect("a component")
            .id
    };
    let yearly = |id, pricing| EditComponent {
        subscription_component_id: id,
        name: None,
        price_entry: PriceEntry::New(PriceInput {
            cadence: BillingPeriodEnum::Annual,
            currency: "EUR".to_string(),
            pricing,
            credit_unit: None,
        }),
    };
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![
                        yearly(id_of("Platform Fee"), Pricing::Rate { rate: dec!(29) }),
                        yearly(
                            id_of("Bandwidth"),
                            Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(0.10) }),
                        ),
                    ],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2025, 2, 11),
        )
        .await
        .expect("the amendment");
    cancel(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(day(2025, 2, 15)),
    )
    .await;
    env.process_cycles().await;

    let year: Vec<String> = documents(&env, sub_id)
        .await
        .into_iter()
        .filter(|line| line.contains("2025-02-11..2026-01-06"))
        .collect();
    assert_eq!(
        year,
        vec![
            "Adjustment 2025-02-11: Platform Fee (adjustment) 2025-02-11..2026-01-06 2614",
            "note on Adjustment 2025-02-11: Platform Fee (adjustment) 2025-02-11..2026-01-06 -2582",
        ]
    );
}

/// 9.99 a year from Jan 25, 2025, renewed Jan 25, 2026 and amended that day to 29.00 a month,
/// cancelled on Jan 27: the month bills its 2 days, 2900 − 2900 × 29/31 = 187, and the year,
/// ended on its first day, gives back all of its 999. Its days are the month's, never its own.
/// Rules: R-080, R-084, R-077
#[tokio::test]
async fn a_year_amended_to_a_month_on_its_first_day_keeps_nothing() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::subscriptions::{
        CancellationEffectiveAt, SubscriptionInterfaceAuto,
    };

    let (env, _) = usage_env().await;
    let yearly = fee_plan(&env, "Yearly", DieselBillingPeriodEnum::Annual, dec!(9.99)).await;
    let sub_id = start(&env, yearly, day(2025, 1, 25)).await;
    env.process_cycles().await;
    let fee = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("the subscription")
        .price_components
        .iter()
        .find(|c| c.name == "Platform Fee")
        .expect("the fee")
        .id;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: fee,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_string(),
                            pricing: Pricing::Rate { rate: dec!(29) },
                            credit_unit: None,
                        }),
                    }],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2026, 1, 25),
        )
        .await
        .expect("the amendment");
    cancel(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(day(2026, 1, 27)),
    )
    .await;
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-25: Platform Fee 2025-01-25..2026-01-25 999",
            "Recurring 2026-01-25: Platform Fee 2026-01-25..2027-01-25 999",
            "note on Recurring 2026-01-25: Platform Fee 2026-01-25..2027-01-25 -999",
            "Adjustment 2026-01-25: Platform Fee 2026-01-25..2026-02-25 2900",
            "note on Adjustment 2026-01-25: Platform Fee 2026-01-25..2026-02-25 -2713",
        ]
    );
}

/// Monthly from Jan 5, 2025; on Feb 7 to a yearly plan (its year to Mar 5), on Feb 13 back to the
/// monthly one, on Mar 6 to the yearly one again: the second switch's origin is its own first
/// renewal, Apr 5, never the first switch's Mar 5. The year then renews on Apr 5, 2026.
/// Rules: R-084
#[tokio::test]
async fn a_second_switch_to_a_cadence_starts_its_own_grid() {
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::{
        AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;
    use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;

    let (env, _) = usage_env().await;
    let monthly = fee_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let yearly = fee_plan(&env, "Yearly", DieselBillingPeriodEnum::Annual, dec!(49.99)).await;
    let sub_id = start(&env, monthly, day(2025, 1, 5)).await;
    env.process_cycles().await;
    change_now(&env, sub_id, yearly, day(2025, 2, 7)).await;
    assert_eq!(period(&env, sub_id).await.1, Some(day(2025, 3, 5)));
    change_now(&env, sub_id, monthly, day(2025, 2, 13)).await;
    let fee = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("the subscription")
        .price_components
        .iter()
        .find(|c| c.name == "Platform Fee")
        .expect("the fee")
        .id;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: fee,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Annual,
                            currency: "EUR".to_string(),
                            pricing: Pricing::Rate { rate: dec!(123.45) },
                            credit_unit: None,
                        }),
                    }],
                    added: vec![],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            day(2025, 2, 18),
        )
        .await
        .expect("the amendment");
    env.process_cycles().await;
    change_now(&env, sub_id, yearly, day(2025, 3, 6)).await;
    assert_eq!(period(&env, sub_id).await.1, Some(day(2025, 4, 5)));
    env.process_cycles().await;
    assert_eq!(
        period(&env, sub_id).await,
        (day(2025, 4, 5), Some(day(2026, 4, 5)))
    );
}

/// 29.00 a month from Jan 1, 2025, renewed Feb 1, cancelled on Feb 3 from Jan 20: January
/// gives back its last 12 days, 2900 × 12/31 = 1123, and February's draft, never lived, is closed
/// empty: no document bills it.
/// Rules: R-077, R-080, R-016
#[tokio::test]
async fn a_cancellation_backdated_before_a_renewal_gives_back_the_period_after_it() {
    use meteroid_store::domain::enums::{InvoiceStatusEnum, InvoiceType};
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let (env, _) = usage_env().await;
    let at = |m: u32, d: u32, h: u32| day(2025, m, d).and_hms_opt(h, 0, 0).unwrap().and_utc();
    env.set_now(at(1, 1, 9));
    let monthly = fee_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let sub_id = start(&env, monthly, day(2025, 1, 1)).await;
    env.set_now(at(2, 1, 9));
    env.process_cycles().await;
    env.set_now(at(2, 3, 9));
    cancel(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(day(2025, 1, 20)),
    )
    .await;
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-01: Platform Fee 2025-01-01..2025-02-01 2900",
            "note on Recurring 2025-01-01: Platform Fee 2025-01-01..2025-02-01 -1123",
        ]
    );
    assert_eq!(
        statuses(&env, sub_id).await,
        vec![
            (
                InvoiceType::Recurring,
                day(2025, 1, 1),
                InvoiceStatusEnum::Finalized
            ),
            (
                InvoiceType::Recurring,
                day(2025, 2, 1),
                InvoiceStatusEnum::Closed
            ),
        ]
    );
}

/// 29.00 a month with bandwidth at 0.10 from Jan 1, 2025; 100 units used on Feb 20 bill 1000 on
/// the Mar 1 renewal, issued. Cancelled on Mar 5 from Feb 15, the usage after the cutoff comes
/// back with the days: February keeps 14 of its 28 days, 2900 × 14/28 = 1450, March gives back
/// all of its 2900, and the 1000 of usage, none of it before Feb 15, comes back whole.
/// Rules: R-077, R-080, R-016
#[tokio::test]
async fn a_cancellation_backdated_across_a_renewal_gives_back_the_usage_after_it() {
    use meteroid_store::domain::enums::InvoiceStatusEnum;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let (env, usage) = usage_env().await;
    let at = |m: u32, d: u32, h: u32| day(2025, m, d).and_hms_opt(h, 0, 0).unwrap().and_utc();
    set_usage(&usage, day(2025, 1, 1), day(2025, 2, 1), 0);
    set_usage(&usage, day(2025, 2, 1), day(2025, 3, 1), 100);
    set_usage(&usage, day(2025, 2, 1), day(2025, 2, 15), 0);
    env.set_now(at(1, 1, 9));
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let sub_id = start(&env, monthly, day(2025, 1, 1)).await;
    env.set_now(at(2, 1, 9));
    env.process_cycles().await;
    env.set_now(at(3, 1, 9));
    env.process_cycles().await;
    for draft in env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .filter(|invoice| invoice.status == InvoiceStatusEnum::Draft)
    {
        env.services()
            .finalize_invoice(Actor::System, draft.id, TENANT_ID)
            .await
            .expect("the renewal finalized");
    }
    env.set_now(at(3, 5, 9));
    cancel(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(day(2025, 2, 15)),
    )
    .await;
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-01: Platform Fee 2025-01-01..2025-02-01 2900",
            "Recurring 2025-02-01: Platform Fee 2025-02-01..2025-03-01 2900",
            "note on Recurring 2025-02-01: Platform Fee 2025-02-01..2025-03-01 -1450",
            "Recurring 2025-03-01: Bandwidth 2025-02-01..2025-03-01 1000",
            "Recurring 2025-03-01: Platform Fee 2025-03-01..2025-04-01 2900",
            "note on Recurring 2025-03-01: Bandwidth 2025-02-01..2025-03-01 -1000",
            "note on Recurring 2025-03-01: Platform Fee 2025-03-01..2025-04-01 -2900",
        ]
    );
}

/// As above with the 100 units used in March, billed 1000 on the Apr 1 renewal, and cancelled on
/// Apr 5 from Feb 15: March is past the ending, its usage owes nothing and comes back whole with
/// the fee, where only the fee came back and the 1000 stayed billed.
/// Rules: R-077, R-080, R-016
#[tokio::test]
async fn a_cancellation_backdated_before_a_whole_period_gives_back_its_usage() {
    use meteroid_store::domain::enums::InvoiceStatusEnum;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let (env, usage) = usage_env().await;
    let at = |m: u32, d: u32, h: u32| day(2025, m, d).and_hms_opt(h, 0, 0).unwrap().and_utc();
    set_usage(&usage, day(2025, 1, 1), day(2025, 2, 1), 0);
    set_usage(&usage, day(2025, 2, 1), day(2025, 3, 1), 0);
    set_usage(&usage, day(2025, 2, 1), day(2025, 2, 15), 0);
    set_usage(&usage, day(2025, 3, 1), day(2025, 4, 1), 100);
    env.set_now(at(1, 1, 9));
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(29)).await;
    let sub_id = start(&env, monthly, day(2025, 1, 1)).await;
    for month in 2..=4 {
        env.set_now(at(month, 1, 9));
        env.process_cycles().await;
    }
    for draft in env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .filter(|invoice| invoice.status == InvoiceStatusEnum::Draft)
    {
        env.services()
            .finalize_invoice(Actor::System, draft.id, TENANT_ID)
            .await
            .expect("the renewal finalized");
    }
    env.set_now(at(4, 5, 9));
    cancel(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(day(2025, 2, 15)),
    )
    .await;
    env.process_cycles().await;

    let notes: Vec<String> = documents(&env, sub_id)
        .await
        .into_iter()
        .filter(|document| document.starts_with("note on Recurring 2025-04-01"))
        .collect();
    assert_eq!(
        notes,
        vec![
            "note on Recurring 2025-04-01: Bandwidth 2025-03-01..2025-04-01 -1000",
            "note on Recurring 2025-04-01: Platform Fee 2025-04-01..2025-05-01 -2900",
        ]
    );
}

/// 9.99 a year from Jan 27, 2025 with 46 units of bandwidth on Feb 1, changed to 123.45 a year
/// on Jan 27, 2026, then cancelled that day from Aug 16, 2025: the year keeps its 201 days,
/// 999 × 201/365 = 550, and the 46 units bill once, 460, on the final invoice issued that day,
/// its line carrying the past period. The renewal still a draft, of a period after the ending,
/// is closed empty; no document is dated before the day it is issued.
/// Rules: R-077, R-080, R-016, R-102
#[tokio::test]
async fn a_cancellation_backdated_before_a_change_bills_its_usage_once_and_gives_the_new_plan_back()
{
    use meteroid_store::domain::enums::{InvoiceStatusEnum, InvoiceType};
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let (env, usage) = usage_env().await;
    let at = |y: i32, m: u32, d: u32, h: u32| day(y, m, d).and_hms_opt(h, 0, 0).unwrap().and_utc();
    set_usage(&usage, day(2025, 1, 27), day(2026, 1, 27), 46);
    set_usage(&usage, day(2025, 1, 27), day(2025, 8, 16), 46);
    env.set_now(at(2025, 1, 27, 9));
    let a = plan(&env, "A", DieselBillingPeriodEnum::Annual, dec!(9.99)).await;
    let b = plan(&env, "B", DieselBillingPeriodEnum::Annual, dec!(123.45)).await;
    let sub_id = start(&env, a, day(2025, 1, 27)).await;
    switch_at_period_end(&env, sub_id, b).await;
    env.set_now(at(2026, 1, 27, 9));
    env.process_cycles().await;
    env.set_now(at(2026, 1, 27, 15));
    cancel(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(day(2025, 8, 16)),
    )
    .await;
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-27: Platform Fee 2025-01-27..2026-01-27 999",
            "note on Recurring 2025-01-27: Platform Fee 2025-01-27..2026-01-27 -449",
            "Final 2026-01-27: Bandwidth 2025-01-27..2025-08-16 460",
        ]
    );
    let issued: Vec<(InvoiceType, NaiveDate, InvoiceStatusEnum)> = env
        .get_invoices(sub_id)
        .await
        .iter()
        .map(|invoice| {
            (
                invoice.invoice_type.clone(),
                invoice.invoice_date,
                invoice.status,
            )
        })
        .collect();
    assert_eq!(
        issued,
        vec![
            (
                InvoiceType::Recurring,
                day(2025, 1, 27),
                InvoiceStatusEnum::Finalized
            ),
            (
                InvoiceType::Recurring,
                day(2026, 1, 27),
                InvoiceStatusEnum::Closed
            ),
            (
                InvoiceType::Final,
                day(2026, 1, 27),
                InvoiceStatusEnum::Finalized
            ),
        ]
    );
}

/// As above, the renewal of Jan 27, 2026 finalized before the cancellation: its line billed the
/// year's 46 units, all before Aug 16, so the cancellation's own invoice bills none again.
/// Rules: R-077, R-080, R-016
#[tokio::test]
async fn a_backdated_cancellation_bills_no_usage_a_finalized_renewal_billed() {
    use meteroid_store::domain::enums::InvoiceStatusEnum;
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let (env, usage) = usage_env().await;
    let at = |y: i32, m: u32, d: u32, h: u32| day(y, m, d).and_hms_opt(h, 0, 0).unwrap().and_utc();
    set_usage(&usage, day(2025, 1, 27), day(2026, 1, 27), 46);
    set_usage(&usage, day(2025, 1, 27), day(2025, 8, 16), 46);
    env.set_now(at(2025, 1, 27, 9));
    let a = plan(&env, "A", DieselBillingPeriodEnum::Annual, dec!(9.99)).await;
    let b = plan(&env, "B", DieselBillingPeriodEnum::Annual, dec!(123.45)).await;
    let sub_id = start(&env, a, day(2025, 1, 27)).await;
    switch_at_period_end(&env, sub_id, b).await;
    env.set_now(at(2026, 1, 27, 9));
    env.process_cycles().await;
    for draft in env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .filter(|invoice| invoice.status == InvoiceStatusEnum::Draft)
    {
        env.services()
            .finalize_invoice(Actor::System, draft.id, TENANT_ID)
            .await
            .expect("the renewal finalized");
    }
    env.set_now(at(2026, 1, 27, 15));
    cancel(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(day(2025, 8, 16)),
    )
    .await;
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-27: Platform Fee 2025-01-27..2026-01-27 999",
            "note on Recurring 2025-01-27: Platform Fee 2025-01-27..2026-01-27 -449",
            "Recurring 2026-01-27: Bandwidth 2025-01-27..2026-01-27 460",
            "Recurring 2026-01-27: Platform Fee 2026-01-27..2027-01-27 12345",
            "note on Recurring 2026-01-27: Platform Fee 2026-01-27..2027-01-27 -12345",
        ]
    );
}

/// 500.00 a month in arrears from Jan 7, 2025, its January billed whole on Feb 7, cancelled
/// that day from Jan 13: January owes its 6 days, 50000 × 6/31 = 9677, so its line gives back
/// 40323, and no other document bills those days again.
/// Rules: R-077, R-080, R-011
#[tokio::test]
async fn a_cancellation_backdated_into_an_arrears_period_billed_gives_back_its_unused_days() {
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let (env, _) = usage_env().await;
    let at = |m: u32, d: u32, h: u32| day(2025, m, d).and_hms_opt(h, 0, 0).unwrap().and_utc();
    env.set_now(at(1, 7, 9));
    let monthly = arrears_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(500)).await;
    let sub_id = start(&env, monthly, day(2025, 1, 7)).await;
    env.set_now(at(2, 7, 9));
    env.process_cycles().await;
    let renewal = invoice_of(&env, sub_id, day(2025, 2, 7)).await;
    env.services()
        .finalize_invoice(Actor::System, renewal.id, TENANT_ID)
        .await
        .expect("the renewal issued");
    env.set_now(at(2, 7, 15));
    cancel(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(day(2025, 1, 13)),
    )
    .await;
    env.process_cycles().await;

    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-02-07: Arrears Fee 2025-01-07..2025-02-07 50000",
            "note on Recurring 2025-02-07: Arrears Fee 2025-01-07..2025-02-07 -40323",
        ]
    );
}

/// As above, its invoices left drafts: the renewal's draft, of a period after the ending, is
/// closed empty, and the final invoice issued on the day of the cancellation bills January's 6
/// days, 9677, once.
/// Rules: R-077, R-080, R-011, R-016
#[tokio::test]
async fn a_cancellation_backdated_into_an_arrears_draft_bills_its_days_once() {
    use meteroid_store::domain::enums::{InvoiceStatusEnum, InvoiceType};
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let (env, _) = usage_env().await;
    let at = |m: u32, d: u32, h: u32| day(2025, m, d).and_hms_opt(h, 0, 0).unwrap().and_utc();
    env.set_now(at(1, 7, 9));
    let monthly = arrears_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(500)).await;
    let sub_id = subscription()
        .plan_version(monthly)
        .start_date(day(2025, 1, 7))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    env.set_now(at(2, 7, 9));
    env.process_cycles().await;
    env.set_now(at(2, 7, 15));
    cancel(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(day(2025, 1, 13)),
    )
    .await;
    env.process_cycles().await;
    for draft in env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .filter(|invoice| invoice.status == InvoiceStatusEnum::Draft)
    {
        env.services()
            .finalize_invoice(Actor::System, draft.id, TENANT_ID)
            .await
            .expect("finalized");
    }

    assert_eq!(
        documents(&env, sub_id).await,
        vec!["Final 2025-02-07: Arrears Fee 2025-01-07..2025-01-13 9677"]
    );
    assert_eq!(
        statuses(&env, sub_id).await,
        vec![
            (
                InvoiceType::Recurring,
                day(2025, 2, 7),
                InvoiceStatusEnum::Closed
            ),
            (
                InvoiceType::Final,
                day(2025, 2, 7),
                InvoiceStatusEnum::Finalized
            ),
        ]
    );
}

/// 500.00 a month in arrears and bandwidth at 0.10 from Jan 7, 2025, 100 units used in January,
/// the Feb 7 renewal not run yet: cancelled on Feb 9 from Feb 7, January's fee and usage, 50000
/// and 1000, go on the final invoice issued Feb 9; no document is dated Feb 7.
/// Rules: R-016, R-011, R-102
#[tokio::test]
async fn a_cancellation_backdated_to_the_period_end_bills_its_arrears_fee_on_the_final_invoice() {
    use meteroid_store::domain::enums::{InvoiceStatusEnum, InvoiceType};
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let (env, usage) = usage_env().await;
    let at = |m: u32, d: u32, h: u32| day(2025, m, d).and_hms_opt(h, 0, 0).unwrap().and_utc();
    set_usage(&usage, day(2025, 1, 7), day(2025, 2, 7), 100);
    env.set_now(at(1, 7, 9));
    let monthly = arrears_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(500)).await;
    let sub_id = start(&env, monthly, day(2025, 1, 7)).await;
    env.set_now(at(2, 9, 9));
    cancel(&env, sub_id, CancellationEffectiveAt::Date(day(2025, 2, 7))).await;
    env.process_cycles().await;
    // Past the invoicing entity's 12 hours of grace.
    env.set_now(at(2, 9, 22));
    env.process_cycles().await;

    let mut billed = documents(&env, sub_id).await;
    billed.sort();
    assert_eq!(
        billed,
        vec![
            "Final 2025-02-09: Arrears Fee 2025-01-07..2025-02-07 50000",
            "Final 2025-02-09: Bandwidth 2025-01-07..2025-02-07 1000",
        ]
    );
    assert_eq!(
        statuses(&env, sub_id).await,
        vec![(
            InvoiceType::Final,
            day(2025, 2, 9),
            InvoiceStatusEnum::Finalized
        )]
    );
}

/// As above, the Feb 7 renewal left a draft billing January: cancelled on Feb 9 from Feb 7, that
/// draft, of a period from the ending, is closed empty, and the final invoice issued Feb 9 bills
/// January's fee and usage, 50000 and 1000, once.
/// Rules: R-016, R-011, R-102
#[tokio::test]
async fn a_cancellation_backdated_to_the_period_end_moves_its_renewal_draft_to_the_final_invoice() {
    use meteroid_store::domain::enums::{InvoiceStatusEnum, InvoiceType};
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let (env, usage) = usage_env().await;
    let at = |m: u32, d: u32, h: u32| day(2025, m, d).and_hms_opt(h, 0, 0).unwrap().and_utc();
    set_usage(&usage, day(2025, 1, 7), day(2025, 2, 7), 100);
    env.set_now(at(1, 7, 9));
    let monthly = arrears_plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(500)).await;
    let sub_id = subscription()
        .plan_version(monthly)
        .start_date(day(2025, 1, 7))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    env.set_now(at(2, 7, 9));
    env.process_cycles().await;
    env.set_now(at(2, 9, 9));
    cancel(&env, sub_id, CancellationEffectiveAt::Date(day(2025, 2, 7))).await;
    env.process_cycles().await;
    for draft in env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .filter(|invoice| invoice.status == InvoiceStatusEnum::Draft)
    {
        env.services()
            .finalize_invoice(Actor::System, draft.id, TENANT_ID)
            .await
            .expect("finalized");
    }

    let mut billed = documents(&env, sub_id).await;
    billed.sort();
    assert_eq!(
        billed,
        vec![
            "Final 2025-02-09: Arrears Fee 2025-01-07..2025-02-07 50000",
            "Final 2025-02-09: Bandwidth 2025-01-07..2025-02-07 1000",
        ]
    );
    assert_eq!(
        statuses(&env, sub_id).await,
        vec![
            (
                InvoiceType::Recurring,
                day(2025, 2, 7),
                InvoiceStatusEnum::Closed
            ),
            (
                InvoiceType::Final,
                day(2025, 2, 9),
                InvoiceStatusEnum::Finalized
            ),
        ]
    );
}

/// Started on Jan 8 on a monthly 123.45 billed in arrears, changed the same day at 15:00 to a
/// yearly 9.99 in advance, cancelled on Jan 10: the change's adjustment bills the year once,
/// 999, and the cancellation gives back its 363 days left, 999 × 363/365 = 994. No other
/// document bills the year, no note corrects one.
/// Rules: R-080, R-011
#[tokio::test]
async fn a_change_on_the_start_day_bills_the_new_fee_once() {
    use chrono::TimeZone;
    let (env, _usage) = usage_env().await;
    let monthly = arrears_plan(
        &env,
        "Monthly arrears",
        DieselBillingPeriodEnum::Monthly,
        dec!(123.45),
    )
    .await;
    let yearly = fee_plan(&env, "Yearly", DieselBillingPeriodEnum::Annual, dec!(9.99)).await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 1, 8, 0, 30, 0).unwrap());
    let sub_id = start(&env, monthly, day(2025, 1, 8)).await;
    env.process_cycles().await;

    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 1, 8, 15, 0, 0).unwrap());
    change_now(&env, sub_id, yearly, day(2025, 1, 8)).await;
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec!["Adjustment 2025-01-08: Platform Fee 2025-01-08..2026-01-08 999"]
    );

    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 1, 10, 15, 0, 0).unwrap());
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            meteroid_store::repositories::subscriptions::CancellationEffectiveAt::Date(day(
                2025, 1, 10,
            )),
        )
        .await
        .expect("cancelled");
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Adjustment 2025-01-08: Platform Fee 2025-01-08..2026-01-08 999",
            "note on Adjustment 2025-01-08: Platform Fee 2025-01-08..2026-01-08 -994",
        ]
    );
}

/// Monthly 49.99 in advance from Jan 7, renewed on Feb 7 and cancelled from that day at 15:00:
/// the cancellation ends it at Feb 7's midnight, so February's period is never used and its
/// invoice is credited whole, 4999. Over its life it bills January alone.
/// Rules: R-016, R-080
#[tokio::test]
async fn a_cancellation_from_the_renewal_day_gives_the_renewal_back() {
    use chrono::TimeZone;
    let (env, _usage) = usage_env().await;
    env.set_consolidate_and_grace(false, 0).await;
    let monthly = fee_plan(
        &env,
        "Monthly",
        DieselBillingPeriodEnum::Monthly,
        dec!(49.99),
    )
    .await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 1, 7, 0, 30, 0).unwrap());
    let sub_id = start(&env, monthly, day(2025, 1, 7)).await;
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 2, 7, 0, 30, 0).unwrap());
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-07: Platform Fee 2025-01-07..2025-02-07 4999",
            "Recurring 2025-02-07: Platform Fee 2025-02-07..2025-03-07 4999",
        ]
    );

    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 2, 7, 15, 0, 0).unwrap());
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            meteroid_store::repositories::subscriptions::CancellationEffectiveAt::Date(day(
                2025, 2, 7,
            )),
        )
        .await
        .expect("cancelled");
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-07: Platform Fee 2025-01-07..2025-02-07 4999",
            "Recurring 2025-02-07: Platform Fee 2025-02-07..2025-03-07 4999",
            "note on Recurring 2025-02-07: Platform Fee 2025-02-07..2025-03-07 -4999",
        ]
    );
}

/// Monthly 123.45 in advance from Jan 15; a one-time fee of 15.00 added on Feb 20 at 16:00, then a
/// change to a yearly plan at 17:00 the same day, which ends every row of the old plan, the
/// one-time one included. The one-time fee bills once, on its day, and is never given back: its
/// row closed on the day it opened still lived that day. The change bills B's 500.00 from Feb 20
/// to the yearly grid's Mar 15 boundary, 50000 × 23/365 = 3151, and gives back A's unused
/// 23 of 28 days, 12345 × 23/28 = 10141.
/// Rules: R-093, R-075, R-080, R-084
#[tokio::test]
async fn a_one_time_fee_ended_on_its_day_by_a_plan_change_stays_billed() {
    use chrono::TimeZone;
    use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum};
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    let (env, _usage) = usage_env().await;
    env.set_consolidate_and_grace(false, 0).await;
    let monthly = fee_plan(
        &env,
        "Monthly",
        DieselBillingPeriodEnum::Monthly,
        dec!(123.45),
    )
    .await;
    let yearly = fee_plan(&env, "Yearly", DieselBillingPeriodEnum::Annual, dec!(500)).await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 1, 15, 0, 30, 0).unwrap());
    let sub_id = start(&env, monthly, day(2025, 1, 15)).await;
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 2, 15, 0, 30, 0).unwrap());
    env.process_cycles().await;

    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 2, 20, 16, 0, 0).unwrap());
    let setup = SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges {
            edited: vec![],
            added: vec![AddExtraComponent {
                name: "Setup".to_owned(),
                product: ProductRef::New {
                    name: "Setup Product".to_owned(),
                    fee_type: FeeTypeEnum::OneTime,
                    fee_structure: FeeStructure::OneTime {},
                },
                price_entry: PriceEntry::New(PriceInput {
                    cadence: BillingPeriodEnum::Monthly,
                    currency: "EUR".to_owned(),
                    pricing: Pricing::OneTime {
                        unit_price: dec!(15),
                        quantity: 1,
                        model: None,
                    },
                    credit_unit: None,
                }),
                fee_structure: None,
            }],
            removed: vec![],
        },
        add_on_changes: AddOnChanges::default(),
    };
    env.services()
        .apply_amendment_immediate_at(Actor::System, sub_id, TENANT_ID, setup, day(2025, 2, 20))
        .await
        .expect("the one-time fee added");
    env.process_cycles().await;

    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 2, 20, 17, 0, 0).unwrap());
    change_now(&env, sub_id, yearly, day(2025, 2, 20)).await;
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-15: Platform Fee 2025-01-15..2025-02-15 12345",
            "Recurring 2025-02-15: Platform Fee 2025-02-15..2025-03-15 12345",
            "note on Recurring 2025-02-15: Platform Fee 2025-02-15..2025-03-15 -10141",
            "Adjustment 2025-02-20: Setup (one-time) 2025-02-20..2025-02-20 1500",
            "Adjustment 2025-02-20: Platform Fee (prorated) 2025-02-20..2025-03-15 3151",
        ]
    );
}

/// Monthly 123.45 in arrears from Jan 8, changed the same day at 15:00 to yearly 9.99 in advance,
/// its fee amended to 123.45 a year on Jan 11, cancelled from Jan 13. The change bills the year,
/// 999; the amendment the difference over the 362 days left, 11346 × 362/365 = 11253; the
/// cancellation gives back the 360 days left of each, 999 × 360/365 = 985 and 11253 × 360/362 =
/// 11191. Over its life: 76. The opening invoice of Jan 8, read at its date, still bills the year
/// whole, but the facts as they stand owe it nothing: no charge is reported left unbilled.
/// Rules: R-077, R-080
#[tokio::test]
async fn a_cancelled_year_amended_once_is_billed_by_its_own_documents() {
    use chrono::TimeZone;
    let (env, _usage) = usage_env().await;
    env.set_consolidate_and_grace(false, 0).await;
    let monthly = arrears_plan(
        &env,
        "Monthly arrears",
        DieselBillingPeriodEnum::Monthly,
        dec!(123.45),
    )
    .await;
    let yearly = plan(&env, "Yearly", DieselBillingPeriodEnum::Annual, dec!(9.99)).await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 1, 8, 0, 30, 0).unwrap());
    let sub_id = start(&env, monthly, day(2025, 1, 8)).await;
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 1, 8, 15, 0, 0).unwrap());
    change_now(&env, sub_id, yearly, day(2025, 1, 8)).await;
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 1, 11, 15, 0, 0).unwrap());
    amend_platform_fee(
        &env,
        sub_id,
        meteroid_store::domain::BillingPeriodEnum::Annual,
        dec!(123.45),
        day(2025, 1, 11),
    )
    .await;
    env.process_cycles().await;

    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 1, 13, 15, 0, 0).unwrap());
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            meteroid_store::repositories::subscriptions::CancellationEffectiveAt::Date(day(
                2025, 1, 13,
            )),
        )
        .await
        .expect("cancelled");
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Adjustment 2025-01-08: Platform Fee 2025-01-08..2026-01-08 999",
            "note on Adjustment 2025-01-08: Platform Fee 2025-01-08..2026-01-08 -985",
            "Adjustment 2025-01-11: Platform Fee (adjustment) 2025-01-11..2026-01-08 11253",
            "note on Adjustment 2025-01-11: Platform Fee (adjustment) 2025-01-11..2026-01-08 -11191",
        ]
    );
}

/// Yearly 29.00 in arrears from Jan 26 2025, renewed Jan 26 2026, changed at 15:00 on Jan 27 to
/// monthly 9.99 in arrears, cancelled from Jan 30. The change is off the yearly grid, so the
/// monthly grid's origin is its first boundary after the change, Feb 26, and the period before it
/// is [Jan 26, Feb 26) (R-084): the new fee's three days bill 999 × 3/31 = 97. The old fee's day
/// of its new year bills 2900 × 1/365 = 8 on the same invoice, the cancellation's.
/// Rules: R-084, R-079, R-016
#[tokio::test]
async fn a_monthly_stub_after_a_yearly_renewal_prices_over_its_grid_period() {
    use chrono::TimeZone;
    let (env, _usage) = usage_env().await;
    env.set_consolidate_and_grace(false, 0).await;
    let yearly = arrears_plan(
        &env,
        "Yearly arrears",
        DieselBillingPeriodEnum::Annual,
        dec!(29),
    )
    .await;
    let monthly = arrears_plan(
        &env,
        "Monthly arrears",
        DieselBillingPeriodEnum::Monthly,
        dec!(9.99),
    )
    .await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 1, 26, 0, 30, 0).unwrap());
    let sub_id = start(&env, yearly, day(2025, 1, 26)).await;
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2026, 1, 26, 0, 30, 0).unwrap());
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2026, 1, 27, 15, 0, 0).unwrap());
    change_now(&env, sub_id, monthly, day(2026, 1, 27)).await;
    env.process_cycles().await;

    env.set_now(chrono::Utc.with_ymd_and_hms(2026, 1, 30, 15, 0, 0).unwrap());
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            meteroid_store::repositories::subscriptions::CancellationEffectiveAt::Date(day(
                2026, 1, 30,
            )),
        )
        .await
        .expect("cancelled");
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2026-01-26: Arrears Fee 2025-01-26..2026-01-26 2900",
            "Recurring 2026-01-30: Arrears Fee 2026-01-27..2026-01-30 97",
            "Recurring 2026-01-30: Arrears Fee 2026-01-26..2026-01-27 8",
        ]
    );
}

/// Yearly 123.45 in arrears from Jan 21, changed on its start day at 15:00 to monthly 500.00 in
/// advance. The change is on the yearly grid's boundary, so the monthly grid's origin is Jan 21
/// (R-084): the change's adjustment bills [Jan 21, Feb 21), 50000, and the Feb 21 renewal
/// [Feb 21, Mar 21), 50000, both periods of that grid. The renewals place the grid from Feb 21,
/// a boundary of the same grid: the same windows, no difference.
/// Rules: R-084
#[tokio::test]
async fn a_cadence_change_on_the_start_day_renews_on_its_grid() {
    use chrono::TimeZone;
    let (env, _usage) = usage_env().await;
    let yearly = arrears_plan(
        &env,
        "Yearly arrears",
        DieselBillingPeriodEnum::Annual,
        dec!(123.45),
    )
    .await;
    let monthly = plan(&env, "Monthly", DieselBillingPeriodEnum::Monthly, dec!(500)).await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 1, 21, 0, 30, 0).unwrap());
    let sub_id = start(&env, yearly, day(2025, 1, 21)).await;
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 1, 21, 15, 0, 0).unwrap());
    change_now(&env, sub_id, monthly, day(2025, 1, 21)).await;
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 2, 21, 0, 30, 0).unwrap());
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Adjustment 2025-01-21: Platform Fee 2025-01-21..2025-02-21 50000",
            "Recurring 2025-02-21: Platform Fee 2025-02-21..2025-03-21 50000",
        ]
    );
}

/// Monthly 9.99 in arrears from Jan 25, changed on Feb 1 to yearly 123.45 in advance, renewed on
/// Feb 25 and cancelled from Mar 3. The change is off the monthly grid, so the yearly origin is
/// Feb 25 and the days before it bill over [Feb 25 2024, Feb 25 2025), 12345 × 24/366 = 810
/// (R-084); the renewal bills the old fee's arrears to the change, 999 × 7/31 = 226, and the
/// year; the cancellation's invoice of Mar 3 bills the year's usage up to it, a window of
/// [Feb 25 2025, Feb 25 2026) cut by the ending, and gives back the 359 days left, 12345 × 359/365
/// = 12142; its 4 units bill 40. That invoice is no renewal: the grid is still the one from
/// Feb 25.
/// Rules: R-084, R-016, R-077
#[tokio::test]
async fn a_cancellation_after_a_cadence_change_bills_on_the_changes_grid() {
    use chrono::TimeZone;
    let (env, usage) = usage_env().await;
    env.set_consolidate_and_grace(false, 0).await;
    let monthly = arrears_plan(
        &env,
        "Monthly arrears",
        DieselBillingPeriodEnum::Monthly,
        dec!(9.99),
    )
    .await;
    let yearly = plan(
        &env,
        "Yearly",
        DieselBillingPeriodEnum::Annual,
        dec!(123.45),
    )
    .await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 1, 25, 0, 30, 0).unwrap());
    let sub_id = start(&env, monthly, day(2025, 1, 25)).await;
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 2, 1, 15, 0, 0).unwrap());
    change_now(&env, sub_id, yearly, day(2025, 2, 1)).await;
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 2, 25, 0, 30, 0).unwrap());
    env.process_cycles().await;
    set_usage(&usage, day(2025, 2, 25), day(2025, 3, 3), 4);

    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 3, 3, 15, 0, 0).unwrap());
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            meteroid_store::repositories::subscriptions::CancellationEffectiveAt::Date(day(
                2025, 3, 3,
            )),
        )
        .await
        .expect("cancelled");
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Adjustment 2025-02-01: Platform Fee (prorated) 2025-02-01..2025-02-25 810",
            "Recurring 2025-02-25: Platform Fee 2025-02-25..2026-02-25 12345",
            "Recurring 2025-02-25: Arrears Fee 2025-01-25..2025-02-01 226",
            "note on Recurring 2025-02-25: Platform Fee 2025-02-25..2026-02-25 -12142",
            "Recurring 2025-03-03: Bandwidth 2025-02-25..2025-03-03 40",
        ]
    );
}

/// Yearly 9.99 in arrears from Jan 24 2025, renewed Jan 24 2026. On Jan 28 a one-time fee of
/// 99.00 is added at 16:00, the plan changed to monthly 9.99 in advance at 17:00 and the
/// subscription cancelled from that day at 18:00: it ends at Jan 28's midnight (R-016), before
/// the day the fee would bill, so the fee is given back, 9900, as the change's 870. What it owes:
/// the year, 999, and Jan 24 to 28 of the next, 999 × 4/365 = 11.
/// Rules: R-016, R-093, R-077
#[tokio::test]
async fn a_one_time_fee_added_on_the_day_a_cancellation_ends_from_is_given_back() {
    use chrono::TimeZone;
    use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum};
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    let (env, _usage) = usage_env().await;
    env.set_consolidate_and_grace(false, 0).await;
    let yearly = arrears_plan(
        &env,
        "Yearly arrears",
        DieselBillingPeriodEnum::Annual,
        dec!(9.99),
    )
    .await;
    let monthly = fee_plan(
        &env,
        "Monthly",
        DieselBillingPeriodEnum::Monthly,
        dec!(9.99),
    )
    .await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2025, 1, 24, 0, 30, 0).unwrap());
    let sub_id = start(&env, yearly, day(2025, 1, 24)).await;
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2026, 1, 24, 0, 30, 0).unwrap());
    env.process_cycles().await;

    env.set_now(chrono::Utc.with_ymd_and_hms(2026, 1, 28, 16, 0, 0).unwrap());
    let setup = SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges {
            edited: vec![],
            added: vec![AddExtraComponent {
                name: "Setup".to_owned(),
                product: ProductRef::New {
                    name: "Setup Product".to_owned(),
                    fee_type: FeeTypeEnum::OneTime,
                    fee_structure: FeeStructure::OneTime {},
                },
                price_entry: PriceEntry::New(PriceInput {
                    cadence: BillingPeriodEnum::Monthly,
                    currency: "EUR".to_owned(),
                    pricing: Pricing::OneTime {
                        unit_price: dec!(99),
                        quantity: 1,
                        model: None,
                    },
                    credit_unit: None,
                }),
                fee_structure: None,
            }],
            removed: vec![],
        },
        add_on_changes: AddOnChanges::default(),
    };
    env.services()
        .apply_amendment_immediate_at(Actor::System, sub_id, TENANT_ID, setup, day(2026, 1, 28))
        .await
        .expect("the one-time fee added");
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2026, 1, 28, 17, 0, 0).unwrap());
    change_now(&env, sub_id, monthly, day(2026, 1, 28)).await;
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2026, 1, 28, 18, 0, 0).unwrap());
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            meteroid_store::repositories::subscriptions::CancellationEffectiveAt::Date(day(
                2026, 1, 28,
            )),
        )
        .await
        .expect("cancelled");
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2026-01-24: Arrears Fee 2025-01-24..2026-01-24 999",
            "Adjustment 2026-01-28: Setup (one-time) 2026-01-28..2026-01-28 9900",
            "note on Adjustment 2026-01-28: Setup (one-time) 2026-01-28..2026-01-28 -9900",
            "Adjustment 2026-01-28: Platform Fee (prorated) 2026-01-28..2026-02-24 870",
            "note on Adjustment 2026-01-28: Platform Fee (prorated) 2026-01-28..2026-02-24 -870",
            "Recurring 2026-01-28: Arrears Fee 2026-01-24..2026-01-28 11",
        ]
    );
}

/// Monthly 500.00 in arrears from Jan 11 2025, switched at the period end to yearly 49.99 in
/// arrears (Mar 11) and back (Mar 11 2026), then on Mar 16 2026 changed to the yearly plan again,
/// off the monthly grid: the yearly origin is Apr 11 2026 (R-084). The Apr 11 2026 renewal bills
/// the monthly fee's five days, 50000 × 5/31 = 8065, and the yearly fee's stub before the origin,
/// 4999 × 26/365 = 356; the Apr 11 2027 renewal its first full year, 4999 (R-011).
/// Rules: R-011, R-084
#[tokio::test]
async fn a_yearly_fee_after_a_stub_bills_its_first_full_year() {
    use chrono::TimeZone;
    let (env, _usage) = usage_env().await;
    env.set_consolidate_and_grace(false, 0).await;
    let monthly = arrears_plan(
        &env,
        "Monthly arrears",
        DieselBillingPeriodEnum::Monthly,
        dec!(500),
    )
    .await;
    let yearly = arrears_plan(
        &env,
        "Yearly arrears",
        DieselBillingPeriodEnum::Annual,
        dec!(49.99),
    )
    .await;
    let at = |y, m, d| chrono::Utc.with_ymd_and_hms(y, m, d, 0, 30, 0).unwrap();
    env.set_now(at(2025, 1, 11));
    let sub_id = start(&env, monthly, day(2025, 1, 11)).await;
    env.process_cycles().await;
    env.set_now(at(2025, 2, 11));
    env.process_cycles().await;
    switch_at_period_end(&env, sub_id, yearly).await;
    env.set_now(at(2025, 3, 11));
    env.process_cycles().await;
    switch_at_period_end(&env, sub_id, monthly).await;
    env.set_now(at(2026, 3, 11));
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2026, 3, 16, 15, 0, 0).unwrap());
    {
        use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
        use meteroid_store::domain::prices::Pricing;
        use meteroid_store::domain::subscription_amendment::{
            AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
        };
        use meteroid_store::domain::subscription_changes::PlanChangeMode;
        let fee = env
            .get_subscription_components(sub_id)
            .await
            .into_iter()
            .find(|c| c.name == "Arrears Fee" && c.effective_to.is_none())
            .expect("the monthly fee")
            .id;
        env.services()
            .apply_amendment_immediate_at(
                Actor::System,
                sub_id,
                TENANT_ID,
                SubscriptionAmendment {
                    apply_mode: PlanChangeMode::Immediate,
                    component_changes: ComponentChanges {
                        edited: vec![EditComponent {
                            subscription_component_id: fee,
                            name: None,
                            price_entry: PriceEntry::New(PriceInput {
                                cadence: meteroid_store::domain::BillingPeriodEnum::Monthly,
                                currency: "EUR".to_string(),
                                pricing: Pricing::ExtraRecurring {
                                    unit_price: dec!(29),
                                    quantity: 1,
                                    model: None,
                                },
                                credit_unit: None,
                            }),
                        }],
                        added: vec![],
                        removed: vec![],
                    },
                    add_on_changes: AddOnChanges::default(),
                },
                day(2026, 3, 16),
            )
            .await
            .expect("the fee amended");
    }
    env.process_cycles().await;
    env.set_now(chrono::Utc.with_ymd_and_hms(2026, 3, 16, 16, 0, 0).unwrap());
    change_now(&env, sub_id, yearly, day(2026, 3, 16)).await;
    env.process_cycles().await;
    env.set_now(at(2026, 4, 11));
    env.process_cycles().await;
    env.set_now(at(2027, 4, 11));
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-02-11: Arrears Fee 2025-01-11..2025-02-11 50000",
            "Recurring 2025-03-11: Arrears Fee 2025-02-11..2025-03-11 50000",
            "Recurring 2026-03-11: Arrears Fee 2025-03-11..2026-03-11 4999",
            "Recurring 2026-04-11: Arrears Fee 2026-03-16..2026-04-11 356",
            "Recurring 2026-04-11: Arrears Fee 2026-03-11..2026-03-16 8065",
            "Recurring 2027-04-11: Arrears Fee 2026-04-11..2027-04-11 4999",
        ]
    );
}

/// 500.00 a year renewing monthly for a 19.00 monthly fee, lowered to 120.00 for Feb 1, the
/// monthly fee removed on Jan 10: the lowering is dated Jan 1, 2026, the year's renewal, Feb 1
/// billing and giving back nothing, where it stayed dated Feb 1 inside the year.
/// Rules: R-106, R-084
#[tokio::test]
async fn an_amendment_for_the_period_end_a_removal_moved_waits_for_the_renewal() {
    use diesel_models::enums::SubscriptionChangeKindEnum;
    use diesel_models::subscription_changes::SubscriptionChangeRow;
    use meteroid_store::domain::BillingPeriodEnum;
    use meteroid_store::domain::enums::FeeTypeEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    let (env, _) = usage_env().await;
    let at = |y: i32, m: u32, d: u32| day(y, m, d).and_hms_opt(9, 0, 0).unwrap().and_utc();
    let amendment =
        |component_changes: ComponentChanges, apply_mode: PlanChangeMode| SubscriptionAmendment {
            apply_mode,
            component_changes,
            add_on_changes: AddOnChanges::default(),
        };
    env.set_now(at(2025, 1, 1));
    let yearly = fee_plan(&env, "Yearly", DieselBillingPeriodEnum::Annual, dec!(500)).await;
    let sub_id = start(&env, yearly, day(2025, 1, 1)).await;
    env.set_now(at(2025, 1, 5));
    let support = AddExtraComponent {
        name: "Support".to_string(),
        product: ProductRef::New {
            name: "Support Product".to_string(),
            fee_type: FeeTypeEnum::Rate,
            fee_structure: FeeStructure::Rate {},
        },
        price_entry: PriceEntry::New(PriceInput {
            cadence: BillingPeriodEnum::Monthly,
            currency: "EUR".to_string(),
            pricing: Pricing::Rate { rate: dec!(19) },
            credit_unit: None,
        }),
        fee_structure: None,
    };
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            amendment(
                ComponentChanges {
                    added: vec![support],
                    ..ComponentChanges::default()
                },
                PlanChangeMode::Immediate,
            ),
            day(2025, 1, 5),
        )
        .await
        .expect("the monthly fee added");
    assert_eq!(
        period(&env, sub_id).await,
        (day(2025, 1, 1), Some(day(2025, 2, 1)))
    );
    let open =
        |components: Vec<diesel_models::subscription_components::SubscriptionComponentRow>,
         name: &str| {
            components
                .into_iter()
                .find(|c| c.name == name && c.effective_to.is_none())
                .expect("the component")
                .id
        };

    env.set_now(at(2025, 1, 6));
    let platform = open(
        env.get_subscription_components(sub_id).await,
        "Platform Fee",
    );
    let event = env
        .services()
        .schedule_amendment(
            Actor::System,
            sub_id,
            TENANT_ID,
            amendment(
                ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: platform,
                        name: None,
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Annual,
                            currency: "EUR".to_string(),
                            pricing: Pricing::Rate { rate: dec!(120) },
                            credit_unit: None,
                        }),
                    }],
                    ..ComponentChanges::default()
                },
                PlanChangeMode::EndOfPeriod,
            ),
        )
        .await
        .expect("the amendment at the period end");
    assert_eq!(event.scheduled_time.date(), day(2025, 2, 1));

    env.set_now(at(2025, 1, 10));
    let support = open(env.get_subscription_components(sub_id).await, "Support");
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            amendment(
                ComponentChanges {
                    removed: vec![support],
                    ..ComponentChanges::default()
                },
                PlanChangeMode::Immediate,
            ),
            day(2025, 1, 10),
        )
        .await
        .expect("the monthly fee removed");
    assert_eq!(
        period(&env, sub_id).await,
        (day(2025, 1, 1), Some(day(2026, 1, 1)))
    );
    let pending = SubscriptionChangeRow::list_pending(
        &mut env.conn().await,
        &TENANT_ID,
        sub_id,
        &[SubscriptionChangeKindEnum::Amendment],
    )
    .await
    .expect("the pending amendment");
    assert_eq!(
        pending
            .iter()
            .map(|change| (change.id, change.effective_at.date()))
            .collect::<Vec<_>>(),
        vec![(event.id, day(2026, 1, 1))]
    );
    let mut rows: Vec<(NaiveDate, Option<NaiveDate>)> = env
        .get_all_subscription_components(sub_id, day(2025, 1, 1), day(2027, 1, 1))
        .await
        .into_iter()
        .filter(|c| c.name == "Platform Fee")
        .map(|c| (c.effective_from, c.effective_to))
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        vec![
            (day(2025, 1, 1), Some(day(2026, 1, 1))),
            (day(2026, 1, 1), None),
        ]
    );

    env.set_now(at(2025, 2, 3));
    env.process_cycles().await;
    env.set_now(at(2026, 1, 2));
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2025-01-01: Platform Fee 2025-01-01..2026-01-01 50000",
            "Adjustment 2025-01-05: Support (prorated) 2025-01-05..2025-02-01 1655",
            "note on Adjustment 2025-01-05: Support (prorated) 2025-01-05..2025-02-01 -1348",
            "Recurring 2026-01-01: Platform Fee 2026-01-01..2027-01-01 12000",
        ]
    );
}
