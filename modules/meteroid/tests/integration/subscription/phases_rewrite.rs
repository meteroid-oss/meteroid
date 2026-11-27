//! One change engine over the timeline (R-162) and the projection it is read with (R-164).
//! Starter is 29.00 + Seats 10.00 a month in advance, renewing on the 1st.

use common_domain::actor::Actor;
use common_domain::ids::SubscriptionId;
use meteroid_store::domain::scheduled_events::PhaseSeats;
use meteroid_store::domain::subscription_changes::{PlanChangeMode, PlanTarget, TargetPrice};
use meteroid_store::domain::subscription_phases::{
    PhaseMinimum, PhaseTerms, RewriteEffect, SubscriptionPhaseInput,
};
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use rstest::rstest;
use rust_decimal_macros::dec;

use super::phases::{at, day, phase, platform_at, ramp, set_phases, starter_from_jan_1 as starter};
use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

pub(super) async fn assert_timeline_replays(env: &TestEnv, sub_id: SubscriptionId) {
    meteroid_testkit::helpers::invariants::assert_timeline_replays(
        env.services(),
        TENANT_ID,
        sub_id,
    )
    .await;
}

async fn totals(
    env: &TestEnv,
    sub_id: SubscriptionId,
    months: u32,
) -> (Vec<(chrono::NaiveDate, i64)>, i64) {
    let projection = env
        .services()
        .project_subscription(TENANT_ID, sub_id, months)
        .await
        .expect("a projection");
    (
        projection
            .invoices
            .iter()
            .map(|invoice| (invoice.on, invoice.total))
            .collect(),
        projection.total,
    )
}

const RAMP_AHEAD: [(u32, i64); 6] = [
    (2, 3900),
    (3, 4190),
    (4, 4190),
    (5, 6509),
    (6, 6509),
    (7, 6509),
];

fn ramp_ahead() -> Vec<(chrono::NaiveDate, i64)> {
    RAMP_AHEAD
        .iter()
        .map(|(m, total)| (day(*m, 1), *total))
        .collect()
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

async fn projected(
    env: &TestEnv,
    sub_id: SubscriptionId,
    months: u32,
) -> Vec<(chrono::NaiveDate, i64)> {
    env.services()
        .project_subscription(TENANT_ID, sub_id, months)
        .await
        .expect("a projection")
        .invoices
        .iter()
        .map(|invoice| (invoice.on, invoice.total))
        .collect()
}

/// Read on a renewal day before the controller bills it, the projection holds that day's
/// invoice; once billed, it holds it no more. A seat bought that day after the renewal is billed
/// at once (2000 − 1000) and adds nothing to the day.
/// Rules: R-164
#[rstest]
#[tokio::test]
async fn a_projection_holds_the_charges_due_today_not_yet_billed(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = starter_from_jan_1(&env).await;

    env.set_now(at(2, 1, 0));
    assert_eq!(
        projected(&env, sub_id, 2).await,
        vec![(day(2, 1), 3900), (day(3, 1), 3900), (day(4, 1), 3900)]
    );

    env.process_cycles().await;
    assert_eq!(
        projected(&env, sub_id, 2).await,
        vec![(day(3, 1), 3900), (day(4, 1), 3900)]
    );
}

/// A coupon of 20 % for three invoices on 49.00 a month: January's invoice takes one use, so
/// the projection from January 15 discounts February and March (4900 − 980 = 3920) and bills
/// April to July whole: 2 × 3920 + 4 × 4900 = 27440. The same once January is finalized.
/// Rules: R-164, R-089
#[rstest]
#[tokio::test]
async fn a_projection_takes_the_coupons_off_the_invoices_they_discount(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let coupon = env
        .create_limited_percentage_coupon("TWENTY3", 20, Some(3))
        .await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .coupon(coupon)
        .create(env.services())
        .await;

    let expected = vec![
        (day(2, 1), 3920),
        (day(3, 1), 3920),
        (day(4, 1), 4900),
        (day(5, 1), 4900),
        (day(6, 1), 4900),
        (day(7, 1), 4900),
    ];
    assert_eq!(projected(&env, sub_id, 6).await, expected);
    let projection = env
        .services()
        .project_subscription(TENANT_ID, sub_id, 6)
        .await
        .expect("a projection");
    assert_eq!(projection.total, 27440);
    assert_eq!(projection.invoices[0].discount, 980);

    env.run_outbox_and_orchestration().await;
    assert_eq!(projected(&env, sub_id, 6).await, expected);
}

/// A minimum of 50.00 a month in March and April, on 39.00 of fees: each period's shortfall
/// with no usage, 1100, is projected on the invoice closing it as a commitment, out of `total`
/// (6 × 3900 = 23400) and in `committed_total` (23400 + 2 × 1100 = 25600).
/// Rules: R-164, R-022
#[rstest]
#[tokio::test]
async fn a_projection_flags_the_commitments_shortfalls(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = starter_from_jan_1(&env).await;
    let phase = |start, label: &str, minimum| SubscriptionPhaseInput {
        start_date: start,
        label: Some(label.to_owned()),
        terms: PhaseTerms {
            minimum,
            ..PhaseTerms::default()
        },
    };
    env.services()
        .set_subscription_phases(
            Actor::System,
            TENANT_ID,
            sub_id,
            vec![
                phase(
                    day(3, 1),
                    "committed",
                    PhaseMinimum::Amount {
                        amount_subunit: 5000,
                        products: vec![],
                    },
                ),
                phase(day(5, 1), "uncommitted", PhaseMinimum::Ended),
            ],
        )
        .await
        .expect("phases set");

    let projection = env
        .services()
        .project_subscription(TENANT_ID, sub_id, 6)
        .await
        .expect("a projection");
    assert_eq!(projection.total, 23400);
    assert_eq!(projection.committed_total, 25600);
    let committed: Vec<_> = projection
        .invoices
        .iter()
        .map(|invoice| (invoice.on, invoice.total, invoice.committed_total))
        .collect();
    assert_eq!(
        committed,
        vec![
            (day(2, 1), 3900, 3900),
            (day(3, 1), 3900, 3900),
            (day(4, 1), 3900, 5000),
            (day(5, 1), 3900, 5000),
            (day(6, 1), 3900, 3900),
            (day(7, 1), 3900, 3900),
        ]
    );
    let shortfall = projection.invoices[2]
        .lines
        .iter()
        .find(|line| line.commitment)
        .expect("a shortfall");
    assert_eq!(
        (shortfall.from, shortfall.until, shortfall.amount),
        (day(3, 1), day(4, 1), 1100)
    );
}

/// A ramp on Starter moved to Pro (99.00 + 25.00 a seat) at its Feb 1 renewal: the plan change
/// is taken, the ramp written again over it. Its platform prices are Starter's, inactive on Pro;
/// its floor of 3 seats holds: 12400 a month, then 9900 + 3 × 2500 = 17400 from May. Cancelled,
/// the ramp bills as before: 3900, 4190, 4190, 6509, 6509, 6509 (31807).
/// Rules: R-162, R-146, R-108
#[rstest]
#[tokio::test]
async fn a_plan_change_writes_the_ramp_again_over_it(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter(&env).await;
    set_phases(&env, starter.sub_id, ramp(&starter))
        .await
        .expect("phases set");

    let (_, timeline) = env
        .services()
        .schedule_plan_change_reporting(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID.into(),
            vec![],
        )
        .await
        .expect("a plan change over the ramp");
    assert_eq!(
        timeline
            .iter()
            .map(|entry| (entry.label.as_str(), entry.effect.clone()))
            .collect::<Vec<_>>(),
        vec![
            (
                "Year 2",
                RewriteEffect::Inactive(vec!["Platform Fee".to_string()])
            ),
            (
                "Year 3",
                RewriteEffect::Inactive(vec!["Platform Fee".to_string()])
            ),
        ]
    );
    assert_eq!(
        totals(&env, starter.sub_id, 6).await,
        (
            vec![
                (day(2, 1), 12400),
                (day(3, 1), 12400),
                (day(4, 1), 12400),
                (day(5, 1), 17400),
                (day(6, 1), 17400),
                (day(7, 1), 17400),
            ],
            89400
        )
    );
    assert_timeline_replays(&env, starter.sub_id).await;

    env.set_now(at(1, 20, 12));
    env.services()
        .cancel_plan_change(Actor::System, starter.sub_id, TENANT_ID)
        .await
        .expect("the plan change withdrawn");
    assert_eq!(totals(&env, starter.sub_id, 6).await, (ramp_ahead(), 31807));
    assert_timeline_replays(&env, starter.sub_id).await;
}

/// The same plan change at once on Jan 16: the adjustment bills Pro's 16 days of January
/// (9900 × 16/31 = 5110, 2500 × 16/31 = 1290) and gives back Starter's (1497, 516); the ramp ahead
/// is written again over Pro, as at the period's end.
/// Rules: R-162, R-077
#[rstest]
#[tokio::test]
async fn an_immediate_plan_change_writes_the_ramp_again_over_it(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter(&env).await;
    set_phases(&env, starter.sub_id, ramp(&starter))
        .await
        .expect("phases set");

    env.set_now(at(1, 16, 12));
    let applied = env
        .services()
        .apply_plan_change_immediate_at(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
            day(1, 16),
        )
        .await
        .expect("an immediate plan change over the ramp");
    let adjustment = env
        .get_invoices(starter.sub_id)
        .await
        .into_iter()
        .find(|invoice| Some(invoice.id) == applied.adjustment_invoice_id)
        .expect("the adjustment");
    let mut lines: Vec<i64> = adjustment
        .line_items
        .iter()
        .map(|line| line.amount_subtotal)
        .collect();
    lines.sort();
    assert_eq!(lines, vec![1290, 5110]);
    assert_eq!(
        totals(&env, starter.sub_id, 6).await.0,
        vec![
            (day(2, 1), 12400),
            (day(3, 1), 12400),
            (day(4, 1), 12400),
            (day(5, 1), 17400),
            (day(6, 1), 17400),
            (day(7, 1), 17400),
        ]
    );
    assert_timeline_replays(&env, starter.sub_id).await;
}

/// An amendment repricing the platform at 25.00 from Feb 1 under a ramp pricing it 31.90 in
/// March and April and flooring 3 seats from May: Feb 1 bills 3500, March and April 4190, May
/// on the amendment's price again, 2500 + 3 × 1000 = 5500: the amendment is the fee's baseline.
/// Rules: R-162, R-146
#[rstest]
#[tokio::test]
async fn an_amendment_is_the_baseline_the_ramp_returns_to(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter(&env).await;
    set_phases(
        &env,
        starter.sub_id,
        vec![
            phase(
                day(3, 1),
                "Year 2",
                PhaseTerms {
                    components: vec![platform_at(&starter, dec!(31.90))],
                    ..PhaseTerms::default()
                },
            ),
            phase(
                day(5, 1),
                "Year 3",
                PhaseTerms {
                    min_seats: vec![PhaseSeats {
                        quantity_id: starter.seats_quantity,
                        code: String::new(),
                        slots: 3,
                    }],
                    ..PhaseTerms::default()
                },
            ),
        ],
    )
    .await
    .expect("phases set");

    let mut amendment = super::phases::edit_price(starter.platform, 2500, false);
    amendment.apply_mode = PlanChangeMode::EndOfPeriod;
    env.services()
        .schedule_amendment(Actor::System, starter.sub_id, TENANT_ID, amendment)
        .await
        .expect("an amendment under the ramp");
    assert_eq!(
        totals(&env, starter.sub_id, 6).await,
        (
            vec![
                (day(2, 1), 3500),
                (day(3, 1), 4190),
                (day(4, 1), 4190),
                (day(5, 1), 5500),
                (day(6, 1), 5500),
                (day(7, 1), 5500),
            ],
            28380
        )
    );
    assert_timeline_replays(&env, starter.sub_id).await;
}

/// A cancellation on Apr 1 replacing a plan change scheduled for Feb 1 writes the ramp again
/// under the withdrawal: February bills Starter, 3900, March Year 2, 4190, then it ends.
/// Rules: R-162, R-141
#[rstest]
#[tokio::test]
async fn a_cancellation_writes_the_ramp_again_under_the_change_it_withdraws(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter(&env).await;
    set_phases(&env, starter.sub_id, ramp(&starter))
        .await
        .expect("phases set");
    env.services()
        .schedule_plan_change(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
        )
        .await
        .expect("Pro from February");

    env.set_now(at(1, 20, 12));
    env.services()
        .cancel_subscription(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(4, 1)),
        )
        .await
        .expect("cancelled on Apr 1");
    assert_eq!(
        totals(&env, starter.sub_id, 6).await.0,
        vec![(day(2, 1), 3900), (day(3, 1), 4190)]
    );
    assert_timeline_replays(&env, starter.sub_id).await;
}

/// A switch already written for Mar 1 (the annual plan at 250.00, 275.00 from its anniversary)
/// stays after a plan change to Pro at Feb 1: February bills Pro, 12400, March the annual plan,
/// 25000. Asked on Feb 15 for Mar 1, the plan change replaces the switch: Pro bills from March.
/// Rules: R-162, R-150
#[rstest]
#[tokio::test]
async fn a_plan_change_keeps_a_later_switch_and_replaces_one_of_its_day(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter(&env).await;
    let (annual, annual_platform) = super::phases::seed_annual_platform(&env).await;
    set_phases(
        &env,
        starter.sub_id,
        vec![phase(
            day(3, 1),
            "Annual",
            super::phases::switch_to(annual, vec![(annual_platform, dec!(250.00))]),
        )],
    )
    .await
    .expect("annual from March");

    env.services()
        .schedule_plan_change(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
        )
        .await
        .expect("Pro from February");
    assert_eq!(
        totals(&env, starter.sub_id, 2).await.0,
        vec![(day(2, 1), 12400), (day(3, 1), 25000)]
    );
    assert_timeline_replays(&env, starter.sub_id).await;
    env.services()
        .cancel_plan_change(Actor::System, starter.sub_id, TENANT_ID)
        .await
        .expect("withdrawn");

    env.run_until(day(2, 15)).await;
    let (_, timeline) = env
        .services()
        .schedule_plan_change_reporting(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID.into(),
            vec![],
        )
        .await
        .expect("Pro from March");
    assert_eq!(timeline[0].effect, RewriteEffect::SwitchReplaced);
    assert_eq!(
        totals(&env, starter.sub_id, 2).await.0,
        vec![(day(3, 1), 12400), (day(4, 1), 12400)]
    );
    assert_timeline_replays(&env, starter.sub_id).await;
}

/// A seat floor of 5 from Mar 1 floors the seat add-on "Seat support" (2.00 a seat) of the same
/// quantity as it floors the plan's seats: March bills 2900 + 5 × 1000 + 5 × 200 = 8900 on 1 seat
/// held. Seats bought below the floor bill nothing; 4 bought on Mar 15 to 7 bill the 17 days of
/// March past the floor, 7000 × 17/31 = 3839 − 2742 = 1097 and 1400 × 17/31 = 768 − 548 = 220,
/// 1317; April bills 2900 + 7000 + 1400 = 11300. A removal below the floor is refused.
/// Rules: R-146, R-137, R-108
#[rstest]
#[tokio::test]
async fn a_seat_floor_floors_seat_add_ons_too(#[future] test_env: TestEnv) {
    use super::seat_quantities::{
        adjustments, change_seats, renewal_on, seat_support, try_change_seats,
    };
    let env = test_env.await;
    env.set_now(at(1, 2, 12));
    let support = seat_support(&env, None).await;
    let sub_id = super::seat_quantities::on_starter(&env).await;
    super::seat_quantities::add_on_at(&env, sub_id, support, day(1, 2)).await;
    set_phases(
        &env,
        sub_id,
        vec![phase(
            day(3, 1),
            "floor",
            PhaseTerms {
                min_seats: vec![PhaseSeats {
                    quantity_id: QUANTITY_SEATS_ID,
                    code: String::new(),
                    slots: 5,
                }],
                ..PhaseTerms::default()
            },
        )],
    )
    .await
    .expect("a floor of 5 from March");
    assert_timeline_replays(&env, sub_id).await;

    env.run_until(day(3, 2)).await;
    assert_eq!(renewal_on(&env, sub_id, day(3, 1)).await.total, 8900);

    let before = adjustments(&env, sub_id).await.len();
    change_seats(&env, sub_id, support, 2, day(3, 10)).await;
    assert_eq!(
        adjustments(&env, sub_id).await.len(),
        before,
        "below the floor"
    );
    change_seats(&env, sub_id, support, 4, day(3, 15)).await;
    let bought = adjustments(&env, sub_id).await;
    let bought = bought
        .iter()
        .find(|invoice| {
            invoice
                .line_items
                .iter()
                .all(|line| line.start_date == day(3, 15))
        })
        .expect("the purchase");
    assert_eq!(bought.total, 1317, "{:?}", bought.line_items);
    assert!(
        try_change_seats(&env, sub_id, support, -3, day(3, 20))
            .await
            .expect_err("below the floor")
            .contains("minimum")
    );

    env.run_until(day(4, 2)).await;
    assert_eq!(renewal_on(&env, sub_id, day(4, 1)).await.total, 11300);
}

/// A plan change sets its own price on a charge of the plan it moves to: Pro's platform at 79.00
/// from Feb 1 bills 7900 + 2500 = 10400, an override row; a price on a charge Pro has not is refused.
/// Rules: R-163
#[rstest]
#[tokio::test]
async fn a_plan_change_prices_the_charges_it_moves_to(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = starter(&env).await.sub_id;
    let unknown = env
        .services()
        .schedule_plan_change(
            Actor::System,
            sub_id,
            TENANT_ID,
            PlanTarget {
                version: PLAN_VERSION_PRO_ID,
                option: None,
                prices: vec![TargetPrice {
                    component_id: COMP_STARTER_PLATFORM_FEE_ID,
                    price: dec!(79.00),
                }],
            },
            vec![],
        )
        .await;
    assert!(format!("{:?}", unknown.expect_err("not Pro's")).contains("no component"));
    env.services()
        .schedule_plan_change(
            Actor::System,
            sub_id,
            TENANT_ID,
            PlanTarget {
                version: PLAN_VERSION_PRO_ID,
                option: None,
                prices: vec![TargetPrice {
                    component_id: COMP_PRO_PLATFORM_FEE_ID,
                    price: dec!(79.00),
                }],
            },
            vec![],
        )
        .await
        .expect("Pro at 79.00");
    assert_eq!(totals(&env, sub_id, 1).await.0, vec![(day(2, 1), 10400)]);
    env.run_until(day(2, 2)).await;
    let platform = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|row| row.name == "Platform Fee" && row.effective_from == day(2, 1))
        .expect("Pro's platform row");
    assert_eq!(platform.price_id, None);
}
