//! Changes inside the period a fixed term's end date cuts: each prices what is left of the
//! period up to the term's end, over the period's full days (R-079, R-080, R-084).
//!
//! Every subscription runs from Jan 1, 2024 to its end date, Feb 15: the Feb 1 renewal bills
//! [Feb 1, Feb 15) of February's 29 days. A share of `[from, until)` is what is left of the price
//! at `from` less what is left at `until`, each rounded once (R-079).

use chrono::{NaiveDate, NaiveTime};
use common_domain::ids::{
    BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId, SubscriptionId,
};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use diesel_models::enums::{CycleActionEnum, SubscriptionStatusEnum};
use meteroid_store::domain::SlotUpgradeBillingMode;
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
};
use meteroid_store::domain::subscription_amendment::{
    AddOnChanges, ComponentChanges, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{SubscriptionAssertExt, TestEnv, subscription, test_env};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).unwrap()
}

/// A subscription to `plan` from Jan 1 to Feb 15, renewed into its cut last period.
async fn renewed_into_the_cut_period(env: &TestEnv, plan: PlanVersionId) -> SubscriptionId {
    let sub_id = subscription()
        .plan_version(plan)
        .start_date(day(1, 1))
        .end_date(day(2, 15))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycles().await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_next_action(Some(CycleActionEnum::EndSubscription))
        .has_period_start(day(2, 1))
        .has_period_end(day(2, 15));
    sub_id
}

/// Every invoice line and credit note line of the subscription, in order.
async fn documents(env: &TestEnv, sub_id: SubscriptionId) -> Vec<String> {
    let mut documents = Vec::new();
    for invoice in env.get_invoices(sub_id).await {
        let mut lines: Vec<String> = invoice
            .line_items
            .iter()
            .map(|line| {
                format!(
                    "{:?} {}: {} {}..{} {}",
                    invoice.invoice_type,
                    invoice.billing_period_start,
                    line.name,
                    line.start_date,
                    line.end_date,
                    line.amount_subtotal
                )
            })
            .collect();
        lines.sort();
        documents.extend(lines);
        for note in env
            .store()
            .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
            .await
            .expect("credit notes")
        {
            let mut lines: Vec<String> = note
                .line_items
                .iter()
                .map(|line| {
                    format!(
                        "note: {} {}..{} {}",
                        line.name, line.start_date, line.end_date, line.amount_subtotal
                    )
                })
                .collect();
            lines.sort();
            documents.extend(lines);
        }
    }
    documents
}

/// An immediate cancellation on Feb 8 gives back the days left up to the term's end.
/// Rules: R-077, R-079, R-080, R-084
#[rstest]
#[tokio::test]
async fn a_cancellation_in_the_cut_period_gives_back_its_days_up_to_the_term_end(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let sub_id = renewed_into_the_cut_period(&env, PLAN_VERSION_1_LEETCODE_ID).await;

    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(2, 8)),
        )
        .await
        .expect("cancel_subscription failed");
    env.process_due_events().await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_status(SubscriptionStatusEnum::Cancelled);

    // Billed [Feb 1, Feb 15): 3500 × 14/29 = 1689.66 → 1690.
    // Used [Feb 1, Feb 8): 3500 − 3500 × 22/29 (2655.17 → 2655) = 845; 1690 − 845 = 845 back.
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Subscription Rate 2024-01-01..2024-02-01 3500",
            "Recurring 2024-02-01: Subscription Rate 2024-02-01..2024-02-15 1690",
            "note: Subscription Rate 2024-02-01..2024-02-15 -845",
        ]
    );
}

/// An immediate upgrade on Feb 8 bills the new prices and gives back the old ones for the 7 days
/// left of the term, each over February's 29.
/// Rules: R-077, R-079, R-080, R-084
#[rstest]
#[tokio::test]
async fn a_plan_change_in_the_cut_period_prices_the_days_up_to_the_term_end(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let sub_id = renewed_into_the_cut_period(&env, PLAN_VERSION_STARTER_ID).await;

    env.services()
        .apply_plan_change_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
            day(2, 8),
        )
        .await
        .expect("apply_plan_change_immediate_at failed");
    env.process_cycles().await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_status(SubscriptionStatusEnum::Completed);

    // Feb 1 bills 2900 × 14/29 = 1400 and 1000 × 14/29 = 482.76 → 483.
    // Starter used [Feb 1, Feb 8): 2900 − 2900 × 22/29 (2200) = 700, so 1400 − 700 = 700 back;
    // 1000 − 1000 × 22/29 (758.62 → 759) = 241, so 483 − 241 = 242 back.
    // Pro bills [Feb 8, Feb 15): 9900 × 22/29 (7510.34 → 7510) − 9900 × 15/29 (5120.69 → 5121)
    // = 2389; 2500 × 22/29 (1896.55 → 1897) − 2500 × 15/29 (1293.10 → 1293) = 604.
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-02-15 1400",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-02-15 483",
            "note: Platform Fee 2024-02-01..2024-02-15 -700",
            "note: Seats 2024-02-01..2024-02-15 -242",
            "Adjustment 2024-02-08: Platform Fee (prorated) 2024-02-08..2024-02-15 2389",
            "Adjustment 2024-02-08: Seats (prorated) 2024-02-08..2024-02-15 604",
        ]
    );
}

/// Two seats added on Feb 8 bill the 7 days left of the term, over February's 29.
/// Rules: R-079, R-084
#[rstest]
#[tokio::test]
async fn a_seat_increase_in_the_cut_period_bills_the_days_up_to_the_term_end(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let sub_id = renewed_into_the_cut_period(&env, PLAN_VERSION_STARTER_ID).await;

    env.services()
        .update_subscription_slots_for_test(
            TENANT_ID,
            sub_id,
            COMP_STARTER_SEATS_ID,
            2,
            SlotUpgradeBillingMode::Optimistic,
            Some(day(2, 8).and_time(NaiveTime::MIN)),
        )
        .await
        .expect("add 2 seats");
    env.process_cycles().await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_status(SubscriptionStatusEnum::Completed);

    // From Feb 8 the fee holds 3 seats for [Feb 8, Feb 15): 3000 × 22/29 (2275.86 → 2276) −
    // 3000 × 15/29 (1551.72 → 1552) = 724, less the seat held, 1000 × 22/29 (758.62 → 759) −
    // 1000 × 15/29 (517.24 → 517) = 242: 482, each total rounded once (R-079).
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-02-15 1400",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-02-15 483",
            "OneOff 2024-02-08: 2 Seats upgrade 2024-02-08..2024-02-15 482",
        ]
    );
}

/// An add-on added on Feb 8 bills the 7 days left of the term; removed on Feb 11 it gives back
/// the 4 it did not use.
/// Rules: R-014, R-078, R-079, R-080, R-084
#[rstest]
#[tokio::test]
async fn an_add_on_added_then_removed_in_the_cut_period_prices_the_days_up_to_the_term_end(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let sub_id = renewed_into_the_cut_period(&env, PLAN_VERSION_STARTER_ID).await;
    let add_on_id = super::amendment::create_rate_addon(&env, "Premium Support", 2000).await;

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
            day(2, 8),
        )
        .await
        .expect("add add-on failed");
    let added = {
        let mut conn = env.conn().await;
        diesel_models::subscription_add_ons::SubscriptionAddOnRow::list_by_subscription_id_active(
            &mut conn, &TENANT_ID, &sub_id, None,
        )
        .await
        .expect("active add-ons")
    };
    env.services()
        .apply_amendment_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![],
                    edited: vec![],
                    removed: vec![added[0].id],
                },
            },
            day(2, 11),
        )
        .await
        .expect("remove add-on failed");
    env.process_cycles().await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_status(SubscriptionStatusEnum::Completed);

    // [Feb 8, Feb 15): 2000 × 22/29 (1517.24 → 1517) − 2000 × 15/29 (1034.48 → 1034) = 483.
    // Used [Feb 8, Feb 11): 1517 − 2000 × 19/29 (1310.34 → 1310) = 207; 483 − 207 = 276 back.
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-02-15 1400",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-02-15 483",
            "Adjustment 2024-02-08: Premium Support (prorated) 2024-02-08..2024-02-15 483",
            "note: Premium Support (prorated) 2024-02-08..2024-02-15 -276",
        ]
    );
}

/// An immediate change to a yearly plan on Feb 8 bills the year's stub up to the term's end, never
/// up to the yearly grid's next boundary past it.
/// Rules: R-079, R-080, R-083, R-084
#[rstest]
#[tokio::test]
async fn a_change_to_a_longer_cadence_in_the_cut_period_bills_up_to_the_term_end(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let annual = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Annual Rate", annual)
        .components(vec![SeedComp::rate(
            PriceComponentId::new(),
            "Platform Fee",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            DieselBillingPeriodEnum::Annual,
            Decimal::new(29_000, 2),
        )])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    let sub_id = renewed_into_the_cut_period(&env, PLAN_VERSION_STARTER_ID).await;

    env.services()
        .apply_plan_change_immediate_at(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            annual,
            vec![],
            day(2, 8),
        )
        .await
        .expect("monthly→annual plan change failed");
    env.process_cycles().await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_status(SubscriptionStatusEnum::Completed);

    // Starter gives back 700 and 242, as for any change on Feb 8.
    // The year runs from Mar 1, the next monthly boundary (R-084): [2023-03-01, 2024-03-01) has
    // 366 days, and the stub [Feb 8, Feb 15) bills 29000 × 22/366 (1743.17 → 1743)
    // − 29000 × 15/366 (1188.52 → 1189) = 554.
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-02-15 1400",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-02-15 483",
            "note: Platform Fee 2024-02-01..2024-02-15 -700",
            "note: Seats 2024-02-01..2024-02-15 -242",
            "Adjustment 2024-02-08: Platform Fee (prorated) 2024-02-08..2024-02-15 554",
        ]
    );
}

/// A change scheduled for the end of the first period applies on the Feb 1 renewal, which bills
/// the new prices for the cut period; one scheduled inside the cut period would apply on the
/// term's end, where nothing is left to bill: it is refused, as one on a cancellation's day.
/// Rules: R-079, R-084, R-092
#[rstest]
#[tokio::test]
async fn a_scheduled_change_on_a_fixed_term_bills_the_new_prices_up_to_the_term_end(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .end_date(day(2, 15))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.services()
        .schedule_plan_change(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
        )
        .await
        .expect("schedule_plan_change failed");
    env.process_cycles().await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_next_action(Some(CycleActionEnum::EndSubscription))
        .has_period_end(day(2, 15));
    let refused = env
        .services()
        .schedule_plan_change(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_STARTER_ID,
            vec![],
        )
        .await
        .expect_err("a change on the term's end is refused");
    assert!(
        format!("{refused:?}").contains("EndsBeforeTheChange"),
        "{refused:?}"
    );
    env.process_cycles().await;
    env.process_due_events().await;
    let sub = env.get_subscription(sub_id).await;
    sub.assert().has_status(SubscriptionStatusEnum::Completed);
    assert_eq!(sub.plan_version_id, PLAN_VERSION_PRO_ID);

    // Pro for [Feb 1, Feb 15): 9900 × 14/29 = 4779.31 → 4779 and 2500 × 14/29 = 1206.90 → 1207.
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-02-15 4779",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-02-15 1207",
        ]
    );
}

/// A cancellation inside a first period shorter than a month, up to the anchor day, gives back
/// its days left over the full month the period is priced over, as its invoice billed it.
/// Rules: R-002, R-077, R-079, R-080, R-084
#[rstest]
#[tokio::test]
async fn a_cancellation_in_a_first_partial_period_gives_back_its_days_over_the_full_month(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(1, 15))
        .billing_day_anchor(1)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_period_start(day(1, 15))
        .has_period_end(day(2, 1));

    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(1, 20)),
        )
        .await
        .expect("cancel_subscription failed");
    env.process_due_events().await;

    // [Jan 15, Feb 1) is 17 of January's 31 days: 3500 × 17/31 = 1919.35 → 1919.
    // Used [Jan 15, Jan 20): 1919 − 3500 × 12/31 (1354.84 → 1355) = 564; 1919 − 564 = 1355 back.
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-15: Subscription Rate 2024-01-15..2024-02-01 1919",
            "note: Subscription Rate 2024-01-15..2024-02-01 -1355",
        ]
    );
}
