//! The test clock lets a subscription live through future months as the workers would see them,
//! with no SQL rewriting its dates.

use chrono::NaiveDate;
use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId};
use diesel_models::enums::{BillingPeriodEnum, SubscriptionStatusEnum};
use meteroid_store::domain::enums::InvoiceStatusEnum;
use rust_decimal::Decimal;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env};

const RATE_CENTS: i64 = 2_000;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

async fn monthly_plan(env: &TestEnv) -> PlanVersionId {
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Monthly", version_id)
        .components(vec![SeedComp::rate(
            PriceComponentId::new(),
            "Platform Fee",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            BillingPeriodEnum::Monthly,
            Decimal::new(RATE_CENTS, 2),
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("seeding the plan");
    version_id
}

/// Started on Jan 31, it renews on the last day of each shorter month; each invoice is dated
/// the day it is issued, once its grace period has passed.
#[tokio::test]
async fn a_subscription_starting_on_the_31st_renews_at_each_month_end() {
    let env = test_env().await;
    let version = monthly_plan(&env).await;
    env.set_now(day(2027, 1, 20).and_hms_opt(9, 0, 0).unwrap().and_utc());

    let id = subscription()
        .plan_version(version)
        .start_date(day(2027, 1, 31))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    assert_eq!(
        env.get_subscription(id).await.status,
        SubscriptionStatusEnum::PendingActivation
    );
    assert!(env.get_invoices(id).await.is_empty());

    env.run_until(day(2027, 5, 2)).await;

    let mut invoices = env.get_invoices(id).await;
    invoices.sort_by_key(|i| i.billing_period_start);
    let issued = |start, on, cycle| {
        (
            start,
            on,
            Some(cycle),
            RATE_CENTS,
            InvoiceStatusEnum::Finalized,
        )
    };
    assert_eq!(
        invoices
            .iter()
            .map(|i| (
                i.billing_period_start,
                i.invoice_date,
                i.cycle_index,
                i.total,
                i.status
            ))
            .collect::<Vec<_>>(),
        vec![
            issued(day(2027, 1, 31), day(2027, 2, 1), 0),
            issued(day(2027, 2, 28), day(2027, 3, 1), 1),
            issued(day(2027, 3, 31), day(2027, 4, 1), 2),
            issued(day(2027, 4, 30), day(2027, 5, 1), 3),
        ],
        "each period is drafted on its first day and issued at the next tick past its grace period"
    );

    let renewed = env.get_subscription(id).await;
    assert_eq!(
        (
            renewed.status,
            renewed.cycle_index,
            renewed.current_period_end
        ),
        (
            SubscriptionStatusEnum::Active,
            Some(3),
            Some(day(2027, 5, 31))
        )
    );
}
