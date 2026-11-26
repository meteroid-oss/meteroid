//! A coupon added to a running subscription discounts its service from its next period: the usage
//! of the period before, billed in arrears after it, keeps its price, so ended that day it takes
//! nothing.
use std::sync::Arc;

use chrono::{NaiveDate, TimeZone, Utc};
use common_domain::actor::Actor;
use common_domain::ids::SubscriptionId;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::enums::ScheduledEventTypeEnum;
use meteroid_store::domain::{Invoice, UsagePeriod};
use meteroid_store::repositories::subscriptions::{
    CancellationEffectiveAt, SubscriptionInterfaceAuto,
};
use rust_decimal::Decimal;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

/// A usage subscription from March 1 with March's usage, and a 20% coupon added on March 10.
async fn running_with_coupon() -> (TestEnv, SubscriptionId) {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    env.set_now(Utc.with_ymd_and_hms(2026, 3, 10, 12, 0, 0).unwrap());
    let sub = subscription()
        .plan_version(PLAN_VERSION_USAGE_ALPHA_ID)
        .start_date(day(3, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycles().await;
    let (start, end) = (
        day(3, 1).and_time(chrono::NaiveTime::MIN),
        day(4, 1).and_time(chrono::NaiveTime::MIN),
    );
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start,
            period_end: end,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod { start, end },
        },
    );
    let coupon = env.create_percentage_coupon("LATER20", 20).await;
    let applied = env
        .services()
        .apply_coupon_to_subscription(Actor::System, TENANT_ID, sub, coupon)
        .await
        .expect("apply the coupon");
    assert_eq!(applied.applied_coupon.effective_from, day(4, 1));
    (env, sub)
}

async fn april_invoice(env: &TestEnv, sub: SubscriptionId) -> Invoice {
    env.set_now(Utc.with_ymd_and_hms(2026, 4, 2, 12, 0, 0).unwrap());
    for _ in 0..10 {
        if env.run_billing_controller().await == 0 {
            break;
        }
    }
    env.get_invoices(sub)
        .await
        .into_iter()
        .filter(|i| i.billing_period_start >= day(3, 2) || i.invoice_date == day(4, 1))
        .max_by_key(|i| i.subtotal)
        .expect("the invoice of April 1")
}

async fn cancel_at_period_end(env: &TestEnv, sub: SubscriptionId) {
    env.services()
        .cancel_subscription(
            Actor::System,
            sub,
            TENANT_ID,
            None,
            CancellationEffectiveAt::EndOfBillingPeriod,
        )
        .await
        .expect("cancel");
}

#[tokio::test]
async fn ending_on_its_start_the_coupon_discounts_nothing() {
    let (env, sub) = running_with_coupon().await;
    cancel_at_period_end(&env, sub).await;

    let april = april_invoice(&env, sub).await;
    assert!(april.subtotal > 0, "March's usage is billed");
    assert_eq!((april.discount, april.coupons.len()), (0, 0));
}

#[tokio::test]
async fn a_cancellation_undone_gives_the_coupon_back() {
    let (env, sub) = running_with_coupon().await;
    cancel_at_period_end(&env, sub).await;
    let event = env
        .store()
        .get_subscription_details(TENANT_ID, sub)
        .await
        .expect("details")
        .pending_events
        .into_iter()
        .find(|e| e.event_type == ScheduledEventTypeEnum::CancelSubscription)
        .expect("the cancellation");
    env.services()
        .cancel_scheduled_event(Actor::System, event.id, sub, TENANT_ID)
        .await
        .expect("undo");

    let april = april_invoice(&env, sub).await;
    let from_april: i64 = april
        .line_items
        .iter()
        .filter(|line| line.start_date >= day(4, 1))
        .map(|line| line.amount_subtotal)
        .sum();
    assert!(from_april > 0 && from_april < april.subtotal, "{april:?}");
    assert_eq!(april.discount, from_april / 5);
}
