//! A change made now, or an ending, withdraws a change written for the period's end first: no two
//! rows of one fee are in force together. Starter: Platform Fee 29.00, Seats 10.00 (one seat);
//! Pro: 99.00 and 25.00; monthly in advance from Jan 1, 2024 (January's 31 days).

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{SubscriptionId, SubscriptionPriceComponentId};
use diesel_models::subscription_changes::SubscriptionChangeRow;
use meteroid_store::domain::enums::BillingPeriodEnum;
use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
use meteroid_store::domain::prices::Pricing;
use meteroid_store::domain::scheduled_events::ScheduledEvent;
use meteroid_store::domain::subscription_amendment::{
    AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use rstest::rstest;
use rust_decimal::Decimal;

use super::derived_state::{assert_billed_once, assert_invariants, documents};
use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).unwrap()
}

fn at(m: u32, d: u32, h: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, m, d, h, 0, 0).unwrap()
}

async fn starter_from_jan_1(env: &TestEnv) -> SubscriptionId {
    subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await
}

async fn platform_fee(env: &TestEnv, sub_id: SubscriptionId) -> SubscriptionPriceComponentId {
    env.get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|row| row.name == "Platform Fee")
        .expect("the Platform Fee in force")
        .id
}

fn reprice_platform(
    mode: PlanChangeMode,
    component: SubscriptionPriceComponentId,
    cents: i64,
) -> SubscriptionAmendment {
    SubscriptionAmendment {
        apply_mode: mode,
        component_changes: ComponentChanges {
            edited: vec![EditComponent {
                subscription_component_id: component,
                name: None,
                price_entry: PriceEntry::New(PriceInput {
                    cadence: BillingPeriodEnum::Monthly,
                    currency: "EUR".to_string(),
                    pricing: Pricing::Rate {
                        rate: Decimal::new(cents, 2),
                    },
                    credit_unit: None,
                }),
            }],
            added: vec![],
            removed: vec![],
        },
        add_on_changes: AddOnChanges::default(),
    }
}

/// The Platform Fee repriced to 39.00 from Feb 1, written as facts on Jan 10.
async fn schedule_platform_at_39(env: &TestEnv, sub_id: SubscriptionId) -> ScheduledEvent {
    env.set_now(at(1, 10, 12));
    let platform = platform_fee(env, sub_id).await;
    env.services()
        .schedule_amendment(
            Actor::System,
            sub_id,
            TENANT_ID,
            reprice_platform(PlanChangeMode::EndOfPeriod, platform, 3900),
        )
        .await
        .expect("an amendment at the period end")
}

async fn withdrawn(env: &TestEnv, event: &ScheduledEvent) -> bool {
    SubscriptionChangeRow::get_by_id(&mut env.conn().await, event.id, &TENANT_ID)
        .await
        .expect("the change")
        .withdrawn_at
        .is_some()
}

/// 39.00 scheduled from Feb 1, then 99.00 from Jan 16: the scheduled change stands, the 99.00
/// written under it up to Feb 1. Jan 16 bills the difference, 7000 × 16/31 = 3612.90 → 3613;
/// Feb 1 bills 39.00.
/// Rules: R-106, R-078, R-080
#[rstest]
#[tokio::test]
async fn an_immediate_amendment_leaves_a_scheduled_one_of_the_same_fee_standing(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let sub_id = starter_from_jan_1(&env).await;
    let scheduled = schedule_platform_at_39(&env, sub_id).await;

    env.set_now(at(1, 16, 12));
    let platform = platform_fee(&env, sub_id).await;
    env.services()
        .apply_amendment_immediate(
            Actor::System,
            sub_id,
            TENANT_ID,
            reprice_platform(PlanChangeMode::Immediate, platform, 9900),
        )
        .await
        .expect("99.00 from Jan 16");
    assert_invariants(&env, "amended").await;

    env.set_now(at(2, 1, 1));
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Adjustment 2024-01-16: Platform Fee (adjustment) 2024-01-16..2024-02-01 3613",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 3900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 1000",
        ]
    );
    assert_eq!(env.get_subscription(sub_id).await.mrr_cents, 3900 + 1000);
    assert_billed_once(&env, sub_id).await;
    // Written as facts, it is never withdrawn.
    assert!(!withdrawn(&env, &scheduled).await);
}

/// 39.00 scheduled from Feb 1, then a one-time 99.00 added on Jan 16: the scheduled change
/// stands. Jan 16 bills 9900 once; Feb 1 bills 39.00.
/// Rules: R-106, R-093
#[rstest]
#[tokio::test]
async fn a_one_time_fee_added_leaves_a_scheduled_amendment_standing(#[future] test_env: TestEnv) {
    use meteroid_store::domain::enums::FeeTypeEnum;
    use meteroid_store::domain::price_components::ProductRef;
    use meteroid_store::domain::prices::FeeStructure;
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };

    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let sub_id = starter_from_jan_1(&env).await;
    let scheduled = schedule_platform_at_39(&env, sub_id).await;

    env.set_now(at(1, 16, 12));
    env.services()
        .apply_amendment_immediate(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    removed: vec![],
                    added: vec![AddExtraComponent {
                        name: "Setup".to_string(),
                        product: ProductRef::New {
                            name: "Setup Product".to_string(),
                            fee_type: FeeTypeEnum::OneTime,
                            fee_structure: FeeStructure::OneTime {},
                        },
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_string(),
                            pricing: Pricing::OneTime {
                                unit_price: Decimal::new(9900, 2),
                                quantity: 1,
                                model: None,
                            },
                            credit_unit: None,
                        }),
                    }],
                },
                add_on_changes: AddOnChanges::default(),
            },
        )
        .await
        .expect("a one-time 99.00 on Jan 16");
    assert_invariants(&env, "amended").await;

    env.set_now(at(2, 1, 1));
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Adjustment 2024-01-16: Setup (one-time) 2024-01-16..2024-01-16 9900",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 3900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 1000",
        ]
    );
    assert_billed_once(&env, sub_id).await;
    // Written as facts, it is never withdrawn.
    assert!(!withdrawn(&env, &scheduled).await);
}

/// 39.00 scheduled from Feb 1, 99.00 from Jan 16 written under it, then the scheduled change
/// withdrawn: the 99.00 runs on. Jan 16 bills 3613; Feb 1 bills 99.00.
/// Rules: R-106
#[rstest]
#[tokio::test]
async fn withdrawing_a_scheduled_amendment_lets_the_one_written_under_it_run_on(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let sub_id = starter_from_jan_1(&env).await;
    schedule_platform_at_39(&env, sub_id).await;

    env.set_now(at(1, 16, 12));
    let platform = platform_fee(&env, sub_id).await;
    env.services()
        .apply_amendment_immediate(
            Actor::System,
            sub_id,
            TENANT_ID,
            reprice_platform(PlanChangeMode::Immediate, platform, 9900),
        )
        .await
        .expect("99.00 from Jan 16");
    env.services()
        .cancel_amendment(Actor::System, sub_id, TENANT_ID)
        .await
        .expect("the scheduled change withdrawn");
    assert_invariants(&env, "withdrawn").await;

    env.set_now(at(2, 1, 1));
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Adjustment 2024-01-16: Platform Fee (adjustment) 2024-01-16..2024-02-01 3613",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 9900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 1000",
        ]
    );
    assert_billed_once(&env, sub_id).await;
}

/// 39.00 scheduled from Feb 1, then Pro from Jan 16, which replaces it. Starter gives back 16/31:
/// 2900 → 1496.77 → 1497, 1000 → 516.13 → 516; Pro bills 16/31: 9900 → 5109.68 → 5110,
/// 2500 → 1290.32 → 1290. Feb 1 bills Pro's month.
/// Rules: R-011, R-080
#[rstest]
#[tokio::test]
async fn an_immediate_plan_change_replaces_a_scheduled_amendment(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let sub_id = starter_from_jan_1(&env).await;
    let scheduled = schedule_platform_at_39(&env, sub_id).await;

    env.set_now(at(1, 16, 12));
    env.services()
        .apply_plan_change_immediate(
            Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
        )
        .await
        .expect("Pro from Jan 16");
    assert_invariants(&env, "changed").await;

    env.set_now(at(2, 1, 1));
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "note: Platform Fee 2024-01-01..2024-02-01 -1497",
            "note: Seats 2024-01-01..2024-02-01 -516",
            "Adjustment 2024-01-16: Platform Fee (prorated) 2024-01-16..2024-02-01 5110",
            "Adjustment 2024-01-16: Seats (prorated) 2024-01-16..2024-02-01 1290",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 9900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 2500",
        ]
    );
    assert_eq!(env.get_subscription(sub_id).await.mrr_cents, 9900 + 2500);
    assert_billed_once(&env, sub_id).await;
    assert!(withdrawn(&env, &scheduled).await);
}

/// 39.00 scheduled from Feb 1, then the subscription cancelled on Jan 21, which withdraws it:
/// 11/31 back, 2900 → 1029.03 → 1029 and 1000 → 354.84 → 355; nothing renews.
/// Rules: R-080
#[rstest]
#[tokio::test]
async fn a_cancellation_withdraws_a_scheduled_amendment(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let sub_id = starter_from_jan_1(&env).await;
    let scheduled = schedule_platform_at_39(&env, sub_id).await;

    env.set_now(at(1, 21, 12));
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(1, 21)),
        )
        .await
        .expect("cancelled on Jan 21");
    env.process_cycles().await;
    env.set_now(at(2, 1, 1));
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "note: Platform Fee 2024-01-01..2024-02-01 -1029",
            "note: Seats 2024-01-01..2024-02-01 -355",
        ]
    );
    assert_invariants(&env, "cancelled").await;
    assert!(withdrawn(&env, &scheduled).await);
}

/// 39.00 scheduled from Feb 1, then the subscription terminated at once on Jan 21: the
/// termination withdraws the amendment, and gives back what the cancellation above does.
/// Rules: R-080
#[rstest]
#[tokio::test]
async fn an_immediate_cancellation_withdraws_a_scheduled_amendment(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let sub_id = starter_from_jan_1(&env).await;
    let scheduled = schedule_platform_at_39(&env, sub_id).await;

    env.set_now(at(1, 21, 0));
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await
        .expect("terminated on Jan 21");
    env.set_now(at(2, 1, 1));
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "note: Platform Fee 2024-01-01..2024-02-01 -1029",
            "note: Seats 2024-01-01..2024-02-01 -355",
        ]
    );
    assert_invariants(&env, "terminated").await;
    assert!(withdrawn(&env, &scheduled).await);
}
