//! A price's rates by dimension change only from a date, as a successor in its lineage (R-097): a fee
//! row prices each day from the price in force that day, a usage period the date splits priced
//! per part from its own usage, and issued lines are never repriced.
use std::collections::HashMap;
use std::sync::Arc;

use chrono::NaiveDate;
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId, ProductId, SubscriptionId,
};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::enums::FeeTypeEnum;
use meteroid_store::domain::price_components::{
    DimensionRate, DimensionRates, MatrixDimension, UsagePricingModel,
};
use meteroid_store::domain::prices::{
    FeeStructure, MatrixDimensionKey, MatrixPriceUpdate, MatrixRowAdd, UsageModel,
};
use meteroid_store::domain::products::ProductNew;
use meteroid_store::domain::{Invoice, UsagePeriod};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::products::ProductInterface;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, m, d).unwrap()
}

fn at(m: u32, d: u32) -> chrono::DateTime<chrono::Utc> {
    day(m, d).and_hms_opt(9, 0, 0).unwrap().and_utc()
}

fn region(value: &str) -> MatrixDimension {
    MatrixDimension {
        key: "region".to_string(),
        value: value.to_string(),
    }
}

/// Bandwidth units per region used over exactly `[from, to)`.
fn set_usage(
    usage: &MutableMockUsageClient,
    from: NaiveDate,
    to: NaiveDate,
    units: &[(&str, i64)],
) {
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
            data: units
                .iter()
                .map(|(name, units)| GroupedUsageData {
                    value: Decimal::from(*units),
                    dimensions: HashMap::from([("region".to_string(), (*name).to_string())]),
                })
                .collect(),
            period: UsagePeriod { start, end },
        },
    );
}

/// A matrix product on bandwidth by region, and a monthly plan billing it: eu at 0.10 a unit.
async fn matrix_plan(env: &TestEnv, name: &'static str) -> (ProductId, PlanVersionId) {
    let product = env
        .store()
        .create_product(
            Actor::System,
            ProductNew {
                name: format!("{name} bandwidth"),
                description: None,
                tenant_id: TENANT_ID,
                family_id: PRODUCT_FAMILY_ID,
                fee_type: FeeTypeEnum::Usage,
                fee_structure: FeeStructure::Usage {
                    metric_id: METRIC_BANDWIDTH,
                    model: UsageModel::DimensionRates,
                },
                catalog: true,
                tax_category_id: None,
                icon: None,
                color: None,
            },
        )
        .await
        .expect("a matrix product")
        .id;
    let version = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), name, version)
        .components(vec![SeedComp::usage(
            PriceComponentId::new(),
            "Bandwidth by region",
            product,
            METRIC_BANDWIDTH,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            UsagePricingModel::DimensionRates(DimensionRates {
                dimensions: vec!["region".to_string()],
                rates: vec![DimensionRate {
                    key: vec!["eu".to_string()],
                    rate: dec!(0.10),
                    label: None,
                }],
                unit_size: Decimal::ONE,
                // As a matrix: usage of a region without a rate is not billed.
                on_unmatched: meteroid_store::domain::rate_cards::RateCardUnmatched::Ignore,
            }),
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    (product, version)
}

async fn edit(
    env: &TestEnv,
    product: ProductId,
    update: MatrixPriceUpdate,
) -> meteroid_store::StoreResult<()> {
    env.services()
        .update_matrix_prices(TENANT_ID, product, update, uuid::Uuid::nil())
        .await
        .map(|_| ())
}

fn add_us(per_unit: Decimal, from: NaiveDate) -> MatrixPriceUpdate {
    MatrixPriceUpdate {
        add_rows: vec![MatrixRowAdd {
            dimension1: region("us"),
            dimension2: None,
            per_unit_prices: HashMap::from([("EUR".to_string(), per_unit)]),
        }],
        remove_rows: vec![],
        effective_on: Some(from),
    }
}

fn remove_eu(from: NaiveDate) -> MatrixPriceUpdate {
    MatrixPriceUpdate {
        add_rows: vec![],
        remove_rows: vec![MatrixDimensionKey {
            dimension1: region("eu"),
            dimension2: None,
        }],
        effective_on: Some(from),
    }
}

fn describe(invoice: &Invoice) -> Vec<String> {
    let of = format!(
        "{:?} {}",
        invoice.invoice_type, invoice.billing_period_start
    );
    let mut lines = Vec::new();
    for line in &invoice.line_items {
        lines.push(format!(
            "{of}: {} {}..{} {}",
            line.name, line.start_date, line.end_date, line.amount_subtotal
        ));
        for sub_line in &line.sub_lines {
            lines.push(format!(
                "{of}:   {} {} x {} = {}",
                sub_line.name, sub_line.quantity, sub_line.unit_price, sub_line.total
            ));
        }
    }
    lines
}

/// Every line and sub-line of the subscription's invoices, in the order issued, and no note.
async fn documents(env: &TestEnv, sub_id: SubscriptionId) -> Vec<String> {
    let mut documents = Vec::new();
    for invoice in env.get_invoices(sub_id).await {
        let notes = env
            .store()
            .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
            .await
            .expect("credit notes");
        assert!(notes.is_empty(), "no note: {notes:?}");
        documents.extend(describe(&invoice));
    }
    documents
}

/// Monthly from Jan 1, eu at 0.10. On Jan 10 a row us at 0.20 is added from Jan 15; on Feb 10, once
/// January's invoice is issued, eu is removed from that day.
/// - January is split on Jan 15: [Jan 1, Jan 15) eu 100 × 0.10 = 1000, us priced by no row; [Jan 15,
///   Feb 1) eu 200 × 0.10 = 2000, us 50 × 0.20 = 1000. Feb 1's invoice bills 1000 + 3000 = 4000.
/// - February is split on Feb 10: [Feb 1, Feb 10) eu 40 × 0.10 = 400, us 10 × 0.20 = 200; [Feb 10,
///   Mar 1) us 20 × 0.20 = 400, eu priced by no row. Mar 1's invoice bills 600 + 400 = 1000.
/// - The January invoice keeps its lines through the second edit.
///
/// Rules: R-097
#[tokio::test]
async fn a_matrix_edit_bills_each_part_of_its_period_at_its_own_rates() {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    let (product, version) = matrix_plan(&env, "Metered").await;
    set_usage(&usage, day(1, 1), day(1, 15), &[("eu", 100), ("us", 100)]);
    set_usage(&usage, day(1, 15), day(2, 1), &[("eu", 200), ("us", 50)]);
    set_usage(&usage, day(2, 1), day(2, 10), &[("eu", 40), ("us", 10)]);
    set_usage(&usage, day(2, 10), day(3, 1), &[("eu", 30), ("us", 20)]);

    env.set_now(at(1, 1));
    let sub_id = subscription()
        .plan_version(version)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    env.set_now(at(1, 10));
    edit(&env, product, add_us(dec!(0.20), day(1, 15)))
        .await
        .expect("us from Jan 15");
    env.run_until(day(2, 2)).await;
    let january = documents(&env, sub_id).await;

    env.set_now(at(2, 10));
    edit(&env, product, remove_eu(day(2, 10)))
        .await
        .expect("eu removed from Feb 10");
    env.run_until(day(3, 2)).await;

    let all = documents(&env, sub_id).await;
    assert_eq!(
        &all[..january.len()],
        &january[..],
        "R-097: issued lines are never repriced"
    );
    assert_eq!(
        all,
        [
            "Recurring 2025-02-01: Bandwidth by region 2025-01-01..2025-01-15 1000",
            "Recurring 2025-02-01:   eu 100 x 0.10 = 1000",
            "Recurring 2025-02-01: Bandwidth by region 2025-01-15..2025-02-01 3000",
            "Recurring 2025-02-01:   eu 200 x 0.10 = 2000",
            "Recurring 2025-02-01:   us 50 x 0.20 = 1000",
            "Recurring 2025-03-01: Bandwidth by region 2025-02-01..2025-02-10 600",
            "Recurring 2025-03-01:   eu 40 x 0.10 = 400",
            "Recurring 2025-03-01:   us 10 x 0.20 = 200",
            "Recurring 2025-03-01: Bandwidth by region 2025-02-10..2025-03-01 400",
            "Recurring 2025-03-01:   us 20 x 0.20 = 400",
        ]
    );
}

/// A date before today is refused, and so is one before a change already dated later; the same
/// day as that change is taken.
///
/// Rules: R-097
#[tokio::test]
async fn a_matrix_edit_dated_before_today_or_a_later_edit_is_refused() {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    let (product, _) = matrix_plan(&env, "Metered").await;
    env.set_now(at(1, 10));
    let refused = edit(&env, product, add_us(dec!(0.20), day(1, 9))).await;
    assert!(
        format!("{refused:?}").contains("a price changes from today or later"),
        "{refused:?}"
    );

    edit(&env, product, add_us(dec!(0.20), day(1, 20)))
        .await
        .expect("us from Jan 20");
    let refused = edit(&env, product, remove_eu(day(1, 15))).await;
    assert!(
        format!("{refused:?}").contains("date this one on or after it"),
        "{refused:?}"
    );
    edit(&env, product, remove_eu(day(1, 20)))
        .await
        .expect("the same day as the last edit");
}

/// Monthly from Jan 1, eu at 0.10, us added at 0.20 from Jan 15; on Jan 20 the plan changes at once
/// to a monthly 31.00 fee. The matrix fee ends on Jan 20, its last period's usage billed per part
/// on Feb 1 (R-011): [Jan 1, Jan 15) eu 100 × 0.10 = 1000; [Jan 15, Jan 20) eu 50 × 0.10 = 500,
/// us 10 × 0.20 = 200. The new fee's stub, Jan 20 to Feb 1, 3100 × 12/31 = 1200, on the change's
/// adjustment; Feb 1 bills February's fee in advance, 3100.
///
/// Rules: R-097, R-011
#[tokio::test]
async fn an_immediate_plan_change_after_a_matrix_edit_bills_each_part_once() {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    let (product, version) = matrix_plan(&env, "Metered").await;
    let flat = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Flat", flat)
        .components(vec![SeedComp::rate(
            PriceComponentId::new(),
            "Flat Fee",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            dec!(31),
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    set_usage(&usage, day(1, 1), day(1, 15), &[("eu", 100), ("us", 100)]);
    set_usage(&usage, day(1, 15), day(1, 20), &[("eu", 50), ("us", 10)]);

    env.set_now(at(1, 1));
    let sub_id = subscription()
        .plan_version(version)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.set_now(at(1, 10));
    edit(&env, product, add_us(dec!(0.20), day(1, 15)))
        .await
        .expect("us from Jan 15");
    env.set_now(at(1, 20));
    env.services()
        .apply_plan_change_immediate_at(Actor::System, sub_id, TENANT_ID, flat, vec![], day(1, 20))
        .await
        .expect("an immediate plan change");
    env.run_until(day(2, 2)).await;

    assert_eq!(
        documents(&env, sub_id).await,
        [
            "Adjustment 2025-01-20: Flat Fee (prorated) 2025-01-20..2025-02-01 1200",
            "Recurring 2025-02-01: Bandwidth by region 2025-01-01..2025-01-15 1000",
            "Recurring 2025-02-01:   eu 100 x 0.10 = 1000",
            "Recurring 2025-02-01: Bandwidth by region 2025-01-15..2025-01-20 700",
            "Recurring 2025-02-01:   eu 50 x 0.10 = 500",
            "Recurring 2025-02-01:   us 10 x 0.20 = 200",
            "Recurring 2025-02-01: Flat Fee 2025-02-01..2025-03-01 3100",
        ]
    );
}

/// Reverting the price lineage while a dated successor exists refuses: dropping it would lose the
/// rates a subscription bills from that date.
#[tokio::test]
async fn the_price_lineage_revert_refuses_while_a_successor_exists() {
    use diesel_async::SimpleAsyncConnection;

    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage)).await;
    let (product, _) = matrix_plan(&env, "Metered").await;
    env.set_now(at(1, 10));
    edit(&env, product, add_us(dec!(0.20), day(1, 15)))
        .await
        .expect("us from Jan 15");

    let down = crate::release_migrations::migration_section(
        crate::release_migrations::BILLING_ON_FACTS_DOWN,
        "Price lineage",
    );
    let mut conn = env.conn().await;
    conn.batch_execute("BEGIN").await.expect("begin");
    let reverted = conn.batch_execute(down).await;
    conn.batch_execute("ROLLBACK").await.expect("rollback");
    assert!(reverted.is_err(), "the revert refuses");
}
