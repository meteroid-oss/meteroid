//! Phases: a ramp written as dated stretches of a subscription, each boundary materialized as the
//! rows billing reads (R-146). Starter is 29.00 + Seats 10.00 a month in advance, renewing on the 1st.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{ScheduledEventId, SubscriptionId, SubscriptionPriceComponentId};
use diesel_models::enums::SubscriptionChangeKindEnum;
use diesel_models::subscription_changes::SubscriptionChangeRow;
use diesel_models::subscription_facts::SubscriptionEndingRow;
use meteroid_store::domain::scheduled_events::PhaseSeats;
use meteroid_store::domain::subscription_amendment::{
    AddOnChanges, ComponentChanges, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::domain::subscription_components::SubscriptionFee;
use meteroid_store::domain::subscription_phases::{
    PhaseComponent, PhaseMinimum, PhaseTerms, RewriteEffect, SubscriptionPhaseInput,
};
use meteroid_store::domain::{
    ComponentParameterization, ComponentParameters, CreateSubscriptionComponents,
};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::subscriptions::{
    CancellationEffectiveAt, SubscriptionInterfaceAuto,
};
use rstest::rstest;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

pub(super) fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).unwrap()
}

pub(super) fn at(m: u32, d: u32, h: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, m, d, h, 0, 0).unwrap()
}

/// Every invoice line of the subscription, in order.
pub(super) async fn documents(env: &TestEnv, sub_id: SubscriptionId) -> Vec<String> {
    let mut invoices = env.get_invoices(sub_id).await;
    invoices.sort_by_key(|invoice| (invoice.billing_period_start, invoice.created_at));
    let mut documents = Vec::new();
    for invoice in invoices {
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
            documents.extend(note.line_items.iter().map(|line| {
                format!(
                    "note: {} {}..{} {}",
                    line.name, line.start_date, line.end_date, line.amount_subtotal
                )
            }));
        }
    }
    documents
}

pub(super) struct Starter {
    pub(super) sub_id: SubscriptionId,
    pub(super) platform: SubscriptionPriceComponentId,
    pub(super) seats_quantity: common_domain::ids::ManagedQuantityId,
}

pub(super) async fn starter_from_jan_1(env: &TestEnv) -> Starter {
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let components = env.get_subscription_components(sub_id).await;
    let named = |name: &str| {
        components
            .iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("a component named {name}"))
    };
    let seats = named("Seats");
    let seats_quantity = seats_quantity_of(env, seats).await;
    Starter {
        sub_id,
        platform: named("Platform Fee").id,
        seats_quantity,
    }
}

/// The managed quantity a seat row counts: its fee's, else the one coded as its unit.
async fn seats_quantity_of(
    env: &TestEnv,
    seats: &diesel_models::subscription_components::SubscriptionComponentRow,
) -> common_domain::ids::ManagedQuantityId {
    let fee = env
        .store()
        .get_subscription_details(TENANT_ID, seats.subscription_id)
        .await
        .expect("details")
        .price_components
        .into_iter()
        .find(|component| component.id == seats.id)
        .expect("the seats in force")
        .fee;
    match fee {
        SubscriptionFee::Slot {
            quantity_id: Some(id),
            ..
        } => id,
        SubscriptionFee::Slot { unit, .. } => {
            diesel_models::managed_quantities::ManagedQuantityRow::find_by_code(
                &mut env.conn().await,
                TENANT_ID,
                &unit,
            )
            .await
            .expect("quantity")
            .expect("the seats' quantity")
            .id
        }
        other => panic!("seats are a slot fee, got {other:?}"),
    }
}

pub(super) fn platform_at(starter: &Starter, price: Decimal) -> PhaseComponent {
    PhaseComponent {
        lineage: Some(starter.platform),
        plan_component: None,
        price,
        list_rate: None,
        name: None,
    }
}

pub(super) fn phase(start: NaiveDate, label: &str, terms: PhaseTerms) -> SubscriptionPhaseInput {
    SubscriptionPhaseInput {
        start_date: start,
        label: Some(label.to_owned()),
        terms,
    }
}

/// Year 2 (here March) +10 % on the platform fee; Year 3 (May) +10 % again and at least 3 seats.
pub(super) fn ramp(starter: &Starter) -> Vec<SubscriptionPhaseInput> {
    vec![
        phase(
            day(3, 1),
            "Year 2",
            PhaseTerms {
                components: vec![platform_at(starter, dec!(31.90))],
                ..PhaseTerms::default()
            },
        ),
        phase(
            day(5, 1),
            "Year 3",
            PhaseTerms {
                components: vec![platform_at(starter, dec!(35.09))],
                min_seats: vec![PhaseSeats {
                    quantity_id: starter.seats_quantity,
                    code: String::new(),
                    slots: 3,
                }],
                ..PhaseTerms::default()
            },
        ),
    ]
}

pub(super) async fn set_phases(
    env: &TestEnv,
    sub_id: SubscriptionId,
    phases: Vec<SubscriptionPhaseInput>,
) -> meteroid_store::StoreResult<Vec<meteroid_store::domain::subscription_phases::SubscriptionPhase>>
{
    env.services()
        .set_subscription_phases(Actor::System, TENANT_ID, sub_id, phases)
        .await
}

async fn pending_phase_changes(env: &TestEnv, sub_id: SubscriptionId) -> Vec<ScheduledEventId> {
    SubscriptionChangeRow::list_pending(
        &mut env.conn().await,
        &TENANT_ID,
        sub_id,
        &[SubscriptionChangeKindEnum::Phase],
    )
    .await
    .expect("pending changes")
    .into_iter()
    .map(|change| change.id)
    .collect()
}

const RAMP_TO_JUNE: [&str; 12] = [
    "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
    "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
    "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
    "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 1000",
    "Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 3190",
    "Recurring 2024-03-01: Seats 2024-03-01..2024-04-01 1000",
    "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 3190",
    "Recurring 2024-04-01: Seats 2024-04-01..2024-05-01 1000",
    "Recurring 2024-05-01: Platform Fee 2024-05-01..2024-06-01 3509",
    "Recurring 2024-05-01: Seats 2024-05-01..2024-06-01 3000",
    "Recurring 2024-06-01: Platform Fee 2024-06-01..2024-07-01 3509",
    "Recurring 2024-06-01: Seats 2024-06-01..2024-07-01 3000",
];

/// Rules: R-146, R-108
#[rstest]
#[tokio::test]
async fn a_ramp_bills_each_phase_from_its_renewal(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;

    let phases = set_phases(&env, starter.sub_id, ramp(&starter))
        .await
        .expect("phases set");
    assert_eq!(
        phases
            .iter()
            .map(|phase| (phase.seq, phase.start_date, phase.end_date))
            .collect::<Vec<_>>(),
        vec![(0, day(3, 1), Some(day(5, 1))), (1, day(5, 1), None)]
    );
    assert_eq!(pending_phase_changes(&env, starter.sub_id).await.len(), 2);

    env.run_until(day(6, 2)).await;
    assert_eq!(documents(&env, starter.sub_id).await, RAMP_TO_JUNE);
    assert!(pending_phase_changes(&env, starter.sub_id).await.is_empty());
}

/// Setting the same phases again writes nothing new; changing the last rewrites it alone.
/// Rules: R-146
#[rstest]
#[tokio::test]
async fn editing_a_later_phase_keeps_the_earlier_ones(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let first = set_phases(&env, starter.sub_id, ramp(&starter))
        .await
        .expect("phases set");
    let again = set_phases(&env, starter.sub_id, ramp(&starter))
        .await
        .expect("the same phases");
    assert_eq!(
        first
            .iter()
            .map(|phase| phase.change_id)
            .collect::<Vec<_>>(),
        again
            .iter()
            .map(|phase| phase.change_id)
            .collect::<Vec<_>>()
    );

    let mut edited = ramp(&starter);
    edited[1].terms.components = vec![platform_at(&starter, dec!(40.00))];
    let edited = set_phases(&env, starter.sub_id, edited)
        .await
        .expect("Year 3 edited");
    assert_eq!(edited[0].change_id, first[0].change_id);
    assert_ne!(edited[1].change_id, first[1].change_id);
    assert_eq!(pending_phase_changes(&env, starter.sub_id).await.len(), 2);

    env.run_until(day(5, 2)).await;
    let lines = documents(&env, starter.sub_id).await;
    assert!(
        lines
            .contains(&"Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 3190".to_owned())
    );
    assert!(
        lines
            .contains(&"Recurring 2024-05-01: Platform Fee 2024-05-01..2024-06-01 4000".to_owned())
    );
    assert!(lines.contains(&"Recurring 2024-05-01: Seats 2024-05-01..2024-06-01 3000".to_owned()));
}

/// Withdrawing every phase brings the subscription back to its own terms.
/// Rules: R-146
#[rstest]
#[tokio::test]
async fn withdrawing_the_phases_restores_the_terms(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    set_phases(&env, starter.sub_id, ramp(&starter))
        .await
        .expect("phases set");
    assert!(
        set_phases(&env, starter.sub_id, vec![])
            .await
            .expect("withdrawn")
            .is_empty()
    );
    assert!(pending_phase_changes(&env, starter.sub_id).await.is_empty());

    env.run_until(day(5, 2)).await;
    let lines = documents(&env, starter.sub_id).await;
    assert!(
        lines
            .contains(&"Recurring 2024-05-01: Platform Fee 2024-05-01..2024-06-01 2900".to_owned())
    );
    assert!(lines.contains(&"Recurring 2024-05-01: Seats 2024-05-01..2024-06-01 1000".to_owned()));
}

/// A cancellation keeps the phases before it, its ending cutting the rest; undoing it brings
/// the whole ramp back.
/// Rules: R-146
#[rstest]
#[tokio::test]
async fn a_cancellation_cuts_the_ramp_and_undoing_it_restores_it(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    set_phases(&env, starter.sub_id, ramp(&starter))
        .await
        .expect("phases set");

    env.services()
        .cancel_subscription(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(7, 1)),
        )
        .await
        .expect("cancelled from July");
    assert_eq!(pending_phase_changes(&env, starter.sub_id).await.len(), 2);
    let ending = SubscriptionEndingRow::list_pending_cancellations(
        &mut env.conn().await,
        TENANT_ID,
        &[starter.sub_id],
    )
    .await
    .expect("pending cancellations")
    .into_iter()
    .next()
    .expect("the cancellation");
    env.services()
        .cancel_scheduled_event(
            Actor::System,
            ScheduledEventId::from(ending.id),
            starter.sub_id,
            TENANT_ID,
        )
        .await
        .expect("cancellation undone");

    env.run_until(day(6, 2)).await;
    assert_eq!(documents(&env, starter.sub_id).await, RAMP_TO_JUNE);
}

/// Cancelled from April, March's phase bills and May's never does: withdrawn at termination.
/// Rules: R-146
#[rstest]
#[tokio::test]
async fn a_termination_withdraws_the_phases_after_it(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    set_phases(&env, starter.sub_id, ramp(&starter))
        .await
        .expect("phases set");
    env.services()
        .cancel_subscription(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(4, 1)),
        )
        .await
        .expect("cancelled from April");

    env.run_until(day(5, 2)).await;
    let lines = documents(&env, starter.sub_id).await;
    assert!(
        lines
            .contains(&"Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 3190".to_owned())
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("2024-04-01..") || line.contains("2024-05-01.."))
    );
    assert!(pending_phase_changes(&env, starter.sub_id).await.is_empty());
    let phases = env
        .services()
        .list_subscription_phases(TENANT_ID, starter.sub_id)
        .await
        .expect("phases");
    assert_eq!(
        phases
            .iter()
            .map(|phase| phase.start_date)
            .collect::<Vec<_>>(),
        vec![day(3, 1)]
    );
}

/// Rules: R-146
#[rstest]
#[tokio::test]
async fn phases_start_on_future_renewals(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let terms = || PhaseTerms {
        components: vec![platform_at(&starter, dec!(31.90))],
        ..PhaseTerms::default()
    };

    let off_grid = set_phases(
        &env,
        starter.sub_id,
        vec![phase(day(3, 15), "mid", terms())],
    )
    .await;
    assert!(format!("{:?}", off_grid.expect_err("not a renewal")).contains("renewal"));
    let past = set_phases(
        &env,
        starter.sub_id,
        vec![phase(day(1, 1), "past", terms())],
    )
    .await;
    assert!(past.is_err());
    let unordered = set_phases(
        &env,
        starter.sub_id,
        vec![
            phase(day(5, 1), "b", terms()),
            phase(day(3, 1), "a", terms()),
        ],
    )
    .await;
    assert!(unordered.is_err());
    assert!(pending_phase_changes(&env, starter.sub_id).await.is_empty());
}

/// One phase's change is never withdrawn alone: it goes with its phase.
/// Rules: R-146
#[rstest]
#[tokio::test]
async fn a_phase_change_is_never_withdrawn_alone(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    set_phases(&env, starter.sub_id, ramp(&starter))
        .await
        .expect("phases set");

    let change = pending_phase_changes(&env, starter.sub_id).await[0];
    let alone = env
        .services()
        .cancel_scheduled_event(Actor::System, change, starter.sub_id, TENANT_ID)
        .await;
    assert!(format!("{:?}", alone.expect_err("a phase's change")).contains("phases"));
    assert_eq!(pending_phase_changes(&env, starter.sub_id).await.len(), 2);
}

/// A seat floor bills at least its seats and never fewer than those held: 5 held bill 5 under a
/// floor of 3, then 8 under a floor of 8.
/// Rules: R-146
#[rstest]
#[tokio::test]
async fn a_seat_floor_bills_at_least_its_seats(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let mut request = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .request();
    request.price_components = Some(CreateSubscriptionComponents {
        parameterized_components: vec![ComponentParameterization {
            component_id: COMP_STARTER_SEATS_ID,
            parameters: ComponentParameters {
                initial_slot_count: Some(5),
                billing_period: None,
                committed_capacity: None,
            },
        }],
        overridden_components: vec![],
        extra_components: vec![],
        remove_components: vec![],
    });
    let sub_id = env
        .services()
        .insert_subscription(Actor::System, request, TENANT_ID)
        .await
        .expect("five seats")
        .id;
    let seats_row = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|row| row.name == "Seats")
        .expect("seats");
    let quantity = seats_quantity_of(&env, &seats_row).await;
    let floor = |slots| PhaseTerms {
        min_seats: vec![PhaseSeats {
            quantity_id: quantity,
            code: String::new(),
            slots,
        }],
        ..PhaseTerms::default()
    };
    set_phases(
        &env,
        sub_id,
        vec![
            phase(day(3, 1), "floor 3", floor(3)),
            phase(day(5, 1), "floor 8", floor(8)),
        ],
    )
    .await
    .expect("phases set");

    env.run_until(day(5, 2)).await;
    let seats: Vec<String> = documents(&env, sub_id)
        .await
        .into_iter()
        .filter(|line| line.contains(": Seats "))
        .collect();
    assert_eq!(
        seats,
        vec![
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 5000",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 5000",
            "Recurring 2024-03-01: Seats 2024-03-01..2024-04-01 5000",
            "Recurring 2024-04-01: Seats 2024-04-01..2024-05-01 5000",
            "Recurring 2024-05-01: Seats 2024-05-01..2024-06-01 8000",
        ]
    );
    let bought = env
        .services()
        .update_subscription_slots(
            TENANT_ID,
            sub_id,
            COMP_STARTER_SEATS_ID,
            1,
            meteroid_store::domain::SlotUpgradeBillingMode::Optimistic,
            Actor::System,
        )
        .await
        .expect("a seat bought under the floor");
    assert_eq!(bought.new_slot_count, 6);
}

/// A phase's minimum binds its periods only: 50.00 from March trues up March and April
/// (39.00 spent each), and ends with May's phase.
/// Rules: R-146, R-022
#[rstest]
#[tokio::test]
async fn a_phase_minimum_binds_its_own_periods(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    set_phases(
        &env,
        starter.sub_id,
        vec![
            phase(
                day(3, 1),
                "committed",
                PhaseTerms {
                    minimum: PhaseMinimum::Amount {
                        amount_subunit: 5000,
                        products: vec![],
                    },
                    ..PhaseTerms::default()
                },
            ),
            phase(
                day(5, 1),
                "uncommitted",
                PhaseTerms {
                    minimum: PhaseMinimum::Ended,
                    ..PhaseTerms::default()
                },
            ),
        ],
    )
    .await
    .expect("phases set");
    let stored: Vec<(NaiveDate, Option<NaiveDate>, i64)> =
        diesel_models::subscription_adjustments::SubscriptionAdjustmentRow::list_minimum_facts(
            &mut env.conn().await,
            starter.sub_id,
        )
        .await
        .expect("minimums")
        .into_iter()
        .map(|row| (row.effective_from, row.effective_to, row.amount_subunit))
        .collect();
    assert_eq!(stored, vec![(day(3, 1), Some(day(5, 1)), 5000)]);

    env.run_until(day(6, 2)).await;
    let minimums: Vec<String> = documents(&env, starter.sub_id)
        .await
        .into_iter()
        .filter(|line| line.to_lowercase().contains("minimum"))
        .collect();
    assert_eq!(
        minimums
            .iter()
            .map(|line| line.rsplit(' ').next().unwrap().to_owned())
            .collect::<Vec<_>>(),
        vec!["1100", "1100"],
        "{minimums:#?}"
    );
    assert!(
        minimums[0].contains("2024-03-01..2024-04-01"),
        "{minimums:#?}"
    );
    assert!(
        minimums[1].contains("2024-04-01..2024-05-01"),
        "{minimums:#?}"
    );
}

async fn item(
    env: &TestEnv,
    sub_id: SubscriptionId,
    input: meteroid_store::domain::subscription_phases::TimelineItemInput,
) -> meteroid_store::StoreResult<meteroid_store::domain::subscription_phases::TimelineItem> {
    env.services()
        .add_timeline_item(Actor::System, TENANT_ID, sub_id, input)
        .await
}

fn sso_lines(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .filter(|line| line.contains(": SSO ") && !line.ends_with(" 0"))
        .collect()
}

/// A free trial of an add-on from today converts to paid on its end: nothing before March, the
/// catalog price from it.
/// Rules: R-147
#[rstest]
#[tokio::test]
async fn a_free_add_on_trial_converts_to_paid_on_its_end(#[future] test_env: TestEnv) {
    use meteroid_store::domain::subscription_phases::{
        ItemEnd, TimelineItemInput, TimelineItemTerms,
    };
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let sso = super::amendment::create_rate_addon(&env, "SSO", 1500).await;
    item(
        &env,
        starter.sub_id,
        TimelineItemInput {
            add_on_id: sso,
            existing: None,
            quantity: 1,
            start_date: day(1, 15),
            until: day(3, 1),
            terms: TimelineItemTerms {
                free: true,
                end: ItemEnd::Convert { price: None },
                prorated: false,
            },
            label: Some("SSO trial".into()),
            reason: None,
        },
    )
    .await
    .expect("a free trial of SSO");

    env.run_until(day(4, 2)).await;
    assert_eq!(
        sso_lines(documents(&env, starter.sub_id).await),
        vec![
            "Recurring 2024-03-01: SSO 2024-03-01..2024-04-01 1500",
            "Recurring 2024-04-01: SSO 2024-04-01..2024-05-01 1500",
        ]
    );
    assert!(pending_phase_changes(&env, starter.sub_id).await.is_empty());
}

/// A paid add-on from a renewal reverts at its end; removing a second one withdraws it whole.
/// Rules: R-147
#[rstest]
#[tokio::test]
async fn a_dated_add_on_bills_its_window_and_a_removed_one_nothing(#[future] test_env: TestEnv) {
    use meteroid_store::domain::subscription_phases::{
        ItemEnd, TimelineItemInput, TimelineItemTerms,
    };
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let sso = super::amendment::create_rate_addon(&env, "SSO", 1500).await;
    let input = |start, until| TimelineItemInput {
        add_on_id: sso,
        existing: None,
        quantity: 1,
        start_date: start,
        until,
        terms: TimelineItemTerms {
            free: false,
            end: ItemEnd::Revert,
            prorated: false,
        },
        label: None,
        reason: None,
    };
    item(&env, starter.sub_id, input(day(2, 1), day(4, 1)))
        .await
        .expect("SSO for February and March");
    let removed = item(&env, starter.sub_id, input(day(5, 1), day(6, 1)))
        .await
        .expect("SSO for May");
    let paid_mid_period = item(&env, starter.sub_id, input(day(1, 15), day(3, 1))).await;
    assert!(format!("{:?}", paid_mid_period.expect_err("paid today")).contains("renewal"));
    env.services()
        .remove_timeline_item(Actor::System, TENANT_ID, starter.sub_id, removed.id)
        .await
        .expect("May's removed");
    assert_eq!(pending_phase_changes(&env, starter.sub_id).await.len(), 2);

    env.run_until(day(6, 2)).await;
    assert_eq!(
        sso_lines(documents(&env, starter.sub_id).await),
        vec![
            "Recurring 2024-02-01: SSO 2024-02-01..2024-03-01 1500",
            "Recurring 2024-03-01: SSO 2024-03-01..2024-04-01 1500",
        ]
    );
}

/// An add-on with an end still to come is never removed by an amendment, and the
/// subscription's termination withdraws its end.
/// Rules: R-147
#[rstest]
#[tokio::test]
async fn a_dated_add_on_is_guarded_and_withdrawn_at_termination(#[future] test_env: TestEnv) {
    use meteroid_store::domain::subscription_phases::{
        ItemEnd, TimelineItemInput, TimelineItemTerms,
    };
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let sso = super::amendment::create_rate_addon(&env, "SSO", 1500).await;
    let dated = item(
        &env,
        starter.sub_id,
        TimelineItemInput {
            add_on_id: sso,
            existing: None,
            quantity: 1,
            start_date: day(2, 1),
            until: day(5, 1),
            terms: TimelineItemTerms {
                free: false,
                end: ItemEnd::Convert {
                    price: Some(dec!(9.00)),
                },
                prorated: false,
            },
            label: None,
            reason: None,
        },
    )
    .await
    .expect("SSO from February");

    let removal = env
        .services()
        .apply_amendment_immediate_at(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![],
                    edited: vec![],
                    removed: vec![dated.lineage],
                },
            },
            day(1, 15),
        )
        .await;
    let refused = format!("{:?}", removal.expect_err("dated add-on"));
    assert!(refused.contains("not found"), "{refused}");

    env.services()
        .cancel_subscription(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(3, 1)),
        )
        .await
        .expect("cancelled from March");
    env.run_until(day(4, 2)).await;
    assert!(pending_phase_changes(&env, starter.sub_id).await.is_empty());
    assert_eq!(
        sso_lines(documents(&env, starter.sub_id).await),
        vec!["Recurring 2024-02-01: SSO 2024-02-01..2024-03-01 1500"]
    );
}

async fn discount(
    env: &TestEnv,
    sub_id: SubscriptionId,
    component: SubscriptionPriceComponentId,
    percent: Decimal,
    start: NaiveDate,
    until: NaiveDate,
) -> meteroid_store::StoreResult<meteroid_store::domain::subscription_phases::DatedDiscount> {
    env.services()
        .add_dated_discount(
            Actor::System,
            TENANT_ID,
            sub_id,
            meteroid_store::domain::subscription_phases::DatedDiscountInput {
                component,
                percent,
                start_date: start,
                until,
                prorated: false,
                label: None,
                reason: None,
            },
        )
        .await
}

/// 20 % off the platform fee for March and April, then its price before; a seats discount
/// removed before its start bills nothing.
/// Rules: R-149
#[rstest]
#[tokio::test]
async fn a_dated_discount_bills_its_window_then_the_price_before(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let seats = env
        .get_subscription_components(starter.sub_id)
        .await
        .into_iter()
        .find(|row| row.name == "Seats")
        .expect("seats")
        .id;
    let platform = discount(
        &env,
        starter.sub_id,
        starter.platform,
        dec!(20),
        day(3, 1),
        day(5, 1),
    )
    .await
    .expect("20 % off in March and April");
    assert_eq!(platform.terms.name.as_deref(), Some("Platform Fee"));
    let removed = discount(&env, starter.sub_id, seats, dec!(50), day(2, 1), day(3, 1))
        .await
        .expect("half off seats in February");
    env.services()
        .remove_timeline_item(Actor::System, TENANT_ID, starter.sub_id, removed.id)
        .await
        .expect("the seats discount removed");

    let mid_period = discount(&env, starter.sub_id, seats, dec!(10), day(2, 15), day(4, 1)).await;
    assert!(format!("{:?}", mid_period.expect_err("not a renewal")).contains("renewal"));
    let too_much = discount(&env, starter.sub_id, seats, dec!(120), day(2, 1), day(4, 1)).await;
    assert!(format!("{:?}", too_much.expect_err("over 100 %")).contains("100%"));

    env.run_until(day(6, 2)).await;
    assert_eq!(
        documents(&env, starter.sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 1000",
            "Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 2320",
            "Recurring 2024-03-01: Seats 2024-03-01..2024-04-01 1000",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 2320",
            "Recurring 2024-04-01: Seats 2024-04-01..2024-05-01 1000",
            "Recurring 2024-05-01: Platform Fee 2024-05-01..2024-06-01 2900",
            "Recurring 2024-05-01: Seats 2024-05-01..2024-06-01 1000",
            "Recurring 2024-06-01: Platform Fee 2024-06-01..2024-07-01 2900",
            "Recurring 2024-06-01: Seats 2024-06-01..2024-07-01 1000",
        ]
    );
    assert!(pending_phase_changes(&env, starter.sub_id).await.is_empty());
}

/// A discounted fee is priced by nothing else until the discount ends, in either order; a
/// started discount stays to its end; the subscription's termination withdraws its end.
/// Rules: R-149
#[rstest]
#[tokio::test]
async fn a_dated_discount_excludes_other_prices_of_its_fee(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    discount(
        &env,
        starter.sub_id,
        starter.platform,
        dec!(20),
        day(2, 1),
        day(4, 1),
    )
    .await
    .expect("20 % off in February and March");

    let again = discount(
        &env,
        starter.sub_id,
        starter.platform,
        dec!(10),
        day(5, 1),
        day(6, 1),
    )
    .await;
    assert!(format!("{:?}", again.expect_err("one at a time")).contains("still to come"));
    let phased = set_phases(&env, starter.sub_id, ramp(&starter)).await;
    assert!(format!("{:?}", phased.expect_err("discounted")).contains("discounted until"));
    // Removing the discounted fee would drop its discount with it (R-162).
    let removal = env
        .services()
        .amendment_timeline_effect(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![],
                    removed: vec![starter.platform],
                },
                add_on_changes: AddOnChanges::default(),
            },
        )
        .await
        .expect("a preview")
        .expect("a timeline after it");
    assert!(
        removal
            .entries
            .iter()
            .any(|entry| matches!(entry.effect, RewriteEffect::Dropped(_))),
        "{:?}",
        removal.entries
    );

    env.run_until(day(2, 2)).await;
    let started = env
        .services()
        .list_dated_discounts(TENANT_ID, starter.sub_id)
        .await
        .expect("discounts")
        .remove(0);
    let removed = env
        .services()
        .remove_timeline_item(Actor::System, TENANT_ID, starter.sub_id, started.id)
        .await;
    assert!(format!("{:?}", removed.expect_err("started")).contains("has started"));

    env.services()
        .cancel_subscription(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(3, 1)),
        )
        .await
        .expect("cancelled from March");
    env.run_until(day(4, 2)).await;
    assert!(pending_phase_changes(&env, starter.sub_id).await.is_empty());
    assert_eq!(
        documents(&env, starter.sub_id)
            .await
            .into_iter()
            .filter(|line| line.contains("Platform Fee"))
            .collect::<Vec<_>>(),
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2320",
        ]
    );
}

/// A phase pricing a fee still to start keeps a discount of it out.
/// Rules: R-149
#[rstest]
#[tokio::test]
async fn a_phased_fee_takes_no_discount(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    set_phases(&env, starter.sub_id, ramp(&starter))
        .await
        .expect("phases set");
    let discounted = discount(
        &env,
        starter.sub_id,
        starter.platform,
        dec!(20),
        day(6, 1),
        day(7, 1),
    )
    .await;
    assert!(format!("{:?}", discounted.expect_err("phased")).contains("still to come"));
}

pub(super) fn edit_price(
    component: SubscriptionPriceComponentId,
    cents: i64,
    slot: bool,
) -> SubscriptionAmendment {
    use meteroid_store::domain::enums::BillingPeriodEnum;
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput};
    use meteroid_store::domain::prices::Pricing;
    use meteroid_store::domain::subscription_amendment::EditComponent;
    let rate = Decimal::new(cents, 2);
    SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges {
            edited: vec![EditComponent {
                subscription_component_id: component,
                name: None,
                price_entry: PriceEntry::New(PriceInput {
                    cadence: BillingPeriodEnum::Monthly,
                    currency: "EUR".to_string(),
                    pricing: if slot {
                        Pricing::Slot {
                            model: meteroid_store::domain::price_components::RatingModel::PerUnit {
                                rate,
                            },
                            min_slots: None,
                            max_slots: None,
                        }
                    } else {
                        Pricing::Rate { rate }
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

/// Boundaries past the evaluate horizon stay on their day: a phase in August, a discount from
/// July to October and a free add-on converting in July, all set in January, bill as set.
/// Rules: R-146, R-147, R-149
#[rstest]
#[tokio::test]
async fn far_dated_timeline_changes_keep_their_day(#[future] test_env: TestEnv) {
    use meteroid_store::domain::subscription_phases::{
        ItemEnd, TimelineItemInput, TimelineItemTerms,
    };
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let seats = env
        .get_subscription_components(starter.sub_id)
        .await
        .into_iter()
        .find(|row| row.name == "Seats")
        .expect("seats")
        .id;
    let sso = super::amendment::create_rate_addon(&env, "SSO", 1500).await;
    set_phases(
        &env,
        starter.sub_id,
        vec![phase(
            day(8, 1),
            "August",
            PhaseTerms {
                components: vec![platform_at(&starter, dec!(31.90))],
                ..PhaseTerms::default()
            },
        )],
    )
    .await
    .expect("a phase in August");
    discount(&env, starter.sub_id, seats, dec!(20), day(7, 1), day(10, 1))
        .await
        .expect("20 % off seats from July");
    item(
        &env,
        starter.sub_id,
        TimelineItemInput {
            add_on_id: sso,
            existing: None,
            quantity: 1,
            start_date: day(1, 15),
            until: day(7, 1),
            terms: TimelineItemTerms {
                free: true,
                end: ItemEnd::Convert { price: None },
                prorated: false,
            },
            label: None,
            reason: None,
        },
    )
    .await
    .expect("SSO free until July");
    assert_eq!(pending_phase_changes(&env, starter.sub_id).await.len(), 4);

    env.run_until(day(10, 2)).await;
    let lines = documents(&env, starter.sub_id).await;
    let of = |name: &str, from: u32| -> Vec<String> {
        lines
            .iter()
            .filter(|line| line.contains(&format!(": {name} ")))
            .filter(|line| {
                line.split_whitespace()
                    .nth(1)
                    .is_some_and(|d| d >= format!("2024-{from:02}-01:").as_str())
            })
            .cloned()
            .collect()
    };
    assert_eq!(
        of("Platform Fee", 7),
        vec![
            "Recurring 2024-07-01: Platform Fee 2024-07-01..2024-08-01 2900",
            "Recurring 2024-08-01: Platform Fee 2024-08-01..2024-09-01 3190",
            "Recurring 2024-09-01: Platform Fee 2024-09-01..2024-10-01 3190",
            "Recurring 2024-10-01: Platform Fee 2024-10-01..2024-11-01 3190",
        ]
    );
    assert_eq!(
        of("Seats", 6),
        vec![
            "Recurring 2024-06-01: Seats 2024-06-01..2024-07-01 1000",
            "Recurring 2024-07-01: Seats 2024-07-01..2024-08-01 800",
            "Recurring 2024-08-01: Seats 2024-08-01..2024-09-01 800",
            "Recurring 2024-09-01: Seats 2024-09-01..2024-10-01 800",
            "Recurring 2024-10-01: Seats 2024-10-01..2024-11-01 1000",
        ]
    );
    assert_eq!(
        sso_lines(lines.clone()),
        vec![
            "Recurring 2024-07-01: SSO 2024-07-01..2024-08-01 1500",
            "Recurring 2024-08-01: SSO 2024-08-01..2024-09-01 1500",
            "Recurring 2024-09-01: SSO 2024-09-01..2024-10-01 1500",
            "Recurring 2024-10-01: SSO 2024-10-01..2024-11-01 1500",
        ]
    );
    assert!(pending_phase_changes(&env, starter.sub_id).await.is_empty());
}

/// Cancelled from April between two phases, then the cancellation undone: May's phase was kept
/// and bills.
/// Rules: R-146
#[rstest]
#[tokio::test]
async fn undoing_a_cancellation_between_phases_keeps_the_later_one(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    set_phases(&env, starter.sub_id, ramp(&starter))
        .await
        .expect("phases set");
    env.services()
        .cancel_subscription(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(4, 1)),
        )
        .await
        .expect("cancelled from April");
    assert_eq!(pending_phase_changes(&env, starter.sub_id).await.len(), 2);
    let ending = SubscriptionEndingRow::list_pending_cancellations(
        &mut env.conn().await,
        TENANT_ID,
        &[starter.sub_id],
    )
    .await
    .expect("pending cancellations")
    .into_iter()
    .next()
    .expect("the cancellation");
    env.services()
        .cancel_scheduled_event(
            Actor::System,
            ScheduledEventId::from(ending.id),
            starter.sub_id,
            TENANT_ID,
        )
        .await
        .expect("cancellation undone");
    env.run_until(day(6, 2)).await;
    assert_eq!(documents(&env, starter.sub_id).await, RAMP_TO_JUNE);
}

/// A fee a timeline change still to come prices is edited now, the timeline written again over
/// the edit: a seat floor in July stays, a discount of a fee the edit replaced is dropped. A
/// phase is written after a scheduled amendment, over its rows.
/// Rules: R-162, R-146, R-149
#[rstest]
#[tokio::test]
async fn edits_ahead_of_the_timeline_rewrite_it(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let seats = env
        .get_subscription_components(starter.sub_id)
        .await
        .into_iter()
        .find(|row| row.name == "Seats")
        .expect("seats")
        .id;
    set_phases(
        &env,
        starter.sub_id,
        vec![phase(
            day(7, 1),
            "floor",
            PhaseTerms {
                min_seats: vec![PhaseSeats {
                    quantity_id: starter.seats_quantity,
                    code: String::new(),
                    slots: 3,
                }],
                ..PhaseTerms::default()
            },
        )],
    )
    .await
    .expect("a floor in July");
    discount(
        &env,
        starter.sub_id,
        starter.platform,
        dec!(20),
        day(3, 1),
        day(5, 1),
    )
    .await
    .expect("20 % off in March and April");

    for (component, slot) in [(seats, true), (starter.platform, false)] {
        env.services()
            .apply_amendment_immediate_at(
                Actor::System,
                starter.sub_id,
                TENANT_ID,
                edit_price(component, 1200, slot),
                day(1, 15),
            )
            .await
            .expect("an edit ahead of the timeline");
    }
    assert_eq!(pending_phase_changes(&env, starter.sub_id).await.len(), 1);
    assert!(
        env.services()
            .list_dated_discounts(TENANT_ID, starter.sub_id)
            .await
            .expect("discounts")
            .is_empty(),
        "the discount of the platform priced before the edit is dropped"
    );
    super::phases_rewrite::assert_timeline_replays(&env, starter.sub_id).await;

    set_phases(&env, starter.sub_id, vec![])
        .await
        .expect("the floor withdrawn");
    let sso = super::amendment::create_rate_addon(&env, "SSO", 1500).await;
    env.services()
        .schedule_amendment(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::EndOfPeriod,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![meteroid_store::domain::CreateSubscriptionAddOn {
                        add_on_id: sso,
                        customization: meteroid_store::domain::SubscriptionAddOnCustomization::None,
                        quantity: 1,
                    }],
                    edited: vec![],
                    removed: vec![],
                },
            },
        )
        .await
        .expect("SSO from February");
    set_phases(
        &env,
        starter.sub_id,
        vec![phase(
            day(7, 1),
            "floor",
            PhaseTerms {
                min_seats: vec![PhaseSeats {
                    quantity_id: starter.seats_quantity,
                    code: String::new(),
                    slots: 3,
                }],
                ..PhaseTerms::default()
            },
        )],
    )
    .await
    .expect("a floor in July, after the amendment of February");
    super::phases_rewrite::assert_timeline_replays(&env, starter.sub_id).await;
}

/// An annual plan seeded beside Starter: one Platform Fee of 290.00 a year on Starter's product.
pub(super) async fn seed_annual_platform(
    env: &TestEnv,
) -> (
    common_domain::ids::PlanVersionId,
    common_domain::ids::PriceComponentId,
) {
    use common_domain::ids::{PlanId, PlanVersionId, PriceComponentId, PriceId};
    use diesel_async::AsyncConnection;
    use uuid::uuid;
    let plan_id = PlanId::from(uuid!("019500b0-0001-7000-8000-000000000001"));
    let version_id = PlanVersionId::from(uuid!("019500b0-0002-7000-8000-000000000001"));
    let component_id = PriceComponentId::from(uuid!("019500b0-0003-7000-8000-000000000001"));
    let price_id = PriceId::from(uuid!("019500b0-0004-7000-8000-000000000001"));
    let mut conn = env.conn().await;
    conn.transaction(async |tx| {
        use diesel_models::errors::DatabaseErrorContainer;
        use diesel_models::plan_versions::PlanVersionRowNew;
        use diesel_models::plans::{PlanRowNew, PlanRowPatch};
        PlanRowNew {
            id: plan_id,
            name: "Annual".to_string(),
            description: None,
            tenant_id: TENANT_ID,
            product_family_id: PRODUCT_FAMILY_ID,
            plan_type: diesel_models::enums::PlanTypeEnum::Standard,
            status: diesel_models::enums::PlanStatusEnum::Active,
        }
        .insert(tx)
        .await?;
        PlanVersionRowNew {
            id: version_id,
            is_draft_version: false,
            plan_id,
            version: 1,
            tenant_id: TENANT_ID,
            period_start_day: None,
            net_terms: 0,
            currency: "EUR".to_string(),
            billing_cycles: None,
            uses_product_pricing: true,
            tax_inclusive: false,
            credit_exhausted_rule: None,
            credit_exhausted_caps: serde_json::json!({}),
        }
        .insert(tx)
        .await?;
        PlanRowPatch {
            id: plan_id,
            tenant_id: TENANT_ID,
            name: None,
            description: None,
            active_version_id: Some(Some(version_id)),
            draft_version_id: None,
            self_service_rank: None,
            issues_license: None,
        }
        .update(tx)
        .await?;
        crate::data::plans::PlanSeed::seed_components(
            tx,
            version_id,
            &[crate::data::plans::SeedComp::rate(
                component_id,
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                price_id,
                diesel_models::enums::BillingPeriodEnum::Annual,
                Decimal::new(29000, 2),
            )],
            "EUR",
        )
        .await?;
        Ok::<(), DatabaseErrorContainer>(())
    })
    .await
    .expect("annual plan seeded");
    (version_id, component_id)
}

/// An annual subscription renewing monthly for a monthly add-on, with a minimum from March: the
/// add-on's removal would put March inside an annual period, and is refused.
/// Rules: R-146, R-084
#[rstest]
#[tokio::test]
async fn a_change_moving_renewals_off_a_phase_is_refused(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let (annual, _) = seed_annual_platform(&env).await;
    let sub_id = subscription()
        .plan_version(annual)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let sso = super::amendment::create_rate_addon(&env, "SSO", 1500).await;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![meteroid_store::domain::CreateSubscriptionAddOn {
                        add_on_id: sso,
                        customization: meteroid_store::domain::SubscriptionAddOnCustomization::None,
                        quantity: 1,
                    }],
                    edited: vec![],
                    removed: vec![],
                },
            },
            day(1, 15),
        )
        .await
        .expect("SSO monthly from today");
    set_phases(
        &env,
        sub_id,
        vec![phase(
            day(3, 1),
            "commit",
            PhaseTerms {
                minimum: PhaseMinimum::Amount {
                    amount_subunit: 50000,
                    products: vec![],
                },
                ..PhaseTerms::default()
            },
        )],
    )
    .await
    .expect("a minimum from March, a monthly renewal");
    let sso_row =
        diesel_models::subscription_add_ons::SubscriptionAddOnRow::list_by_subscription_id_active(
            &mut env.conn().await,
            &TENANT_ID,
            &sub_id,
            None,
        )
        .await
        .expect("add-ons")
        .into_iter()
        .find(|row| row.add_on_id == sso)
        .expect("the SSO row")
        .id;
    let removal = env
        .services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![],
                    edited: vec![],
                    removed: vec![sso_row],
                },
            },
            day(1, 20),
        )
        .await;
    let refused = format!("{:?}", removal.expect_err("renewals moved"));
    assert!(refused.contains("is not one"), "{refused}");
    assert_eq!(pending_phase_changes(&env, sub_id).await.len(), 1);
}

pub(super) fn year(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

pub(super) fn switch_to(
    plan_version_id: common_domain::ids::PlanVersionId,
    prices: Vec<(common_domain::ids::PriceComponentId, Decimal)>,
) -> PhaseTerms {
    PhaseTerms {
        components: prices
            .into_iter()
            .map(|(plan_component, price)| PhaseComponent {
                lineage: None,
                plan_component: Some(plan_component),
                price,
                list_rate: None,
                name: None,
            })
            .collect(),
        plan: Some(meteroid_store::domain::subscription_phases::PhasePlan {
            plan_version_id,
            option: None,
            component_params: vec![],
            name: None,
        }),
        ..PhaseTerms::default()
    }
}

/// A monthly pilot on Starter, then the annual plan from March at 250.00 negotiated, then 275.00
/// from its first anniversary, named by its plan component before its row exists.
/// Rules: R-150, R-146
#[rstest]
#[tokio::test]
async fn a_phase_switches_plan_and_cadence(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let (annual, annual_platform) = seed_annual_platform(&env).await;
    let contract = |uplift_on: NaiveDate| {
        vec![
            phase(
                day(3, 1),
                "Annual",
                switch_to(annual, vec![(annual_platform, dec!(250.00))]),
            ),
            phase(
                uplift_on,
                "Year 2",
                PhaseTerms {
                    components: vec![PhaseComponent {
                        lineage: None,
                        plan_component: Some(annual_platform),
                        price: dec!(275.00),
                        list_rate: None,
                        name: None,
                    }],
                    ..PhaseTerms::default()
                },
            ),
        ]
    };

    let off_cadence = set_phases(&env, starter.sub_id, contract(day(6, 1))).await;
    assert!(
        format!("{:?}", off_cadence.expect_err("monthly no more")).contains("next: 2025-03-01")
    );
    let phases = set_phases(&env, starter.sub_id, contract(year(2025, 3, 1)))
        .await
        .expect("pilot, then annual");

    let alone = env
        .services()
        .cancel_scheduled_event(
            Actor::System,
            phases[0].change_id,
            starter.sub_id,
            TENANT_ID,
        )
        .await;
    assert!(format!("{:?}", alone.expect_err("a phase's switch")).contains("phases"));

    env.run_until(year(2025, 3, 2)).await;
    assert_eq!(
        documents(&env, starter.sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 1000",
            "Recurring 2024-03-01: Platform Fee 2024-03-01..2025-03-01 25000",
            "Recurring 2025-03-01: Platform Fee 2025-03-01..2026-03-01 27500",
        ]
    );
    assert_eq!(
        env.get_subscription(starter.sub_id).await.plan_version_id,
        annual
    );
}

/// A cancellation keeps a switch still to come with its phase; the termination before it
/// withdraws both, and the pilot ends on its plan.
/// Rules: R-150
#[rstest]
#[tokio::test]
async fn a_termination_withdraws_a_switch_still_to_come(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let (annual, annual_platform) = seed_annual_platform(&env).await;
    set_phases(
        &env,
        starter.sub_id,
        vec![phase(
            day(3, 1),
            "Annual",
            switch_to(annual, vec![(annual_platform, dec!(250.00))]),
        )],
    )
    .await
    .expect("pilot, then annual");
    env.services()
        .cancel_subscription(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(2, 1)),
        )
        .await
        .expect("cancelled from February");
    assert_eq!(
        env.services()
            .list_subscription_phases(TENANT_ID, starter.sub_id)
            .await
            .expect("phases")
            .len(),
        1
    );
    env.run_until(day(4, 2)).await;
    assert!(
        env.services()
            .list_subscription_phases(TENANT_ID, starter.sub_id)
            .await
            .expect("phases")
            .is_empty()
    );
    assert_eq!(
        documents(&env, starter.sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
        ]
    );
}

/// Terms are absolute: May's phase prices nothing and floors no seats, so March's price and
/// floor end with it and May bills Starter's own 29.00 and one seat.
/// Rules: R-146
#[rstest]
#[tokio::test]
async fn a_phase_leaving_a_fee_out_bills_it_as_before_the_phases(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    set_phases(
        &env,
        starter.sub_id,
        vec![
            phase(
                day(3, 1),
                "Promo",
                PhaseTerms {
                    components: vec![platform_at(&starter, dec!(31.90))],
                    min_seats: vec![PhaseSeats {
                        quantity_id: starter.seats_quantity,
                        code: String::new(),
                        slots: 3,
                    }],
                    ..PhaseTerms::default()
                },
            ),
            phase(day(5, 1), "List", PhaseTerms::default()),
        ],
    )
    .await
    .expect("a promo, then list");
    env.run_until(day(5, 2)).await;
    assert_eq!(
        documents(&env, starter.sub_id).await[4..],
        [
            "Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 3190",
            "Recurring 2024-03-01: Seats 2024-03-01..2024-04-01 3000",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 3190",
            "Recurring 2024-04-01: Seats 2024-04-01..2024-05-01 3000",
            "Recurring 2024-05-01: Platform Fee 2024-05-01..2024-06-01 2900",
            "Recurring 2024-05-01: Seats 2024-05-01..2024-06-01 1000",
        ]
    );
}

/// A phase from March 15, inside March: settled on its day as an immediate amendment, the rest
/// of March at 31.90 and the 29.00 billed for it given back; withdrawing a second one before its
/// day leaves nothing to settle.
/// Rules: R-151, R-120
#[rstest]
#[tokio::test]
async fn a_phase_inside_a_period_settles_on_its_day(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let prorated = |start, price| {
        phase(
            start,
            "mid",
            PhaseTerms {
                components: vec![platform_at(&starter, price)],
                prorated: true,
                ..PhaseTerms::default()
            },
        )
    };
    set_phases(
        &env,
        starter.sub_id,
        vec![
            prorated(day(3, 15), dec!(31.90)),
            prorated(day(4, 20), dec!(35.00)),
        ],
    )
    .await
    .expect("phases inside periods");
    set_phases(
        &env,
        starter.sub_id,
        vec![prorated(day(3, 15), dec!(31.90))],
    )
    .await
    .expect("April's withdrawn");
    let unflagged = set_phases(
        &env,
        starter.sub_id,
        vec![phase(
            day(3, 15),
            "mid",
            PhaseTerms {
                components: vec![platform_at(&starter, dec!(31.90))],
                ..PhaseTerms::default()
            },
        )],
    )
    .await;
    assert!(format!("{:?}", unflagged.expect_err("not a renewal")).contains("renewal"));

    env.run_until(day(5, 2)).await;
    let lines: Vec<String> = documents(&env, starter.sub_id)
        .await
        .into_iter()
        .filter(|line| line.contains("Platform Fee"))
        .collect();
    assert_eq!(
        lines,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
            "Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 2900",
            "Adjustment 2024-03-15: Platform Fee (adjustment) 2024-03-15..2024-04-01 159",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 3190",
            "Recurring 2024-05-01: Platform Fee 2024-05-01..2024-06-01 3190",
        ]
    );
    assert!(pending_phase_changes(&env, starter.sub_id).await.is_empty());
}

/// 20 % off the platform fee from March 15 to April 15, both inside a period: each boundary
/// settles on its day, each side prorated by its days and rounded on its own (1272 − 1590 in
/// March, 1547 − 1237 in April).
/// Rules: R-151, R-149
#[rstest]
#[tokio::test]
async fn a_discount_inside_periods_settles_each_boundary(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let refused = discount(
        &env,
        starter.sub_id,
        starter.platform,
        dec!(20),
        day(3, 15),
        day(4, 15),
    )
    .await;
    assert!(format!("{:?}", refused.expect_err("not flagged")).contains("renewal"));
    env.services()
        .add_dated_discount(
            Actor::System,
            TENANT_ID,
            starter.sub_id,
            meteroid_store::domain::subscription_phases::DatedDiscountInput {
                component: starter.platform,
                percent: dec!(20),
                start_date: day(3, 15),
                until: day(4, 15),
                prorated: true,
                label: None,
                reason: None,
            },
        )
        .await
        .expect("20 % off from mid-March to mid-April");
    env.run_until(day(5, 2)).await;
    let lines: Vec<String> = documents(&env, starter.sub_id)
        .await
        .into_iter()
        .filter(|line| line.contains("Platform Fee"))
        .collect();
    assert_eq!(
        lines,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
            "Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 2900",
            "note: Platform Fee 2024-03-01..2024-04-01 -318",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 2320",
            "Adjustment 2024-04-15: Platform Fee (adjustment) 2024-04-15..2024-05-01 310",
            "Recurring 2024-05-01: Platform Fee 2024-05-01..2024-06-01 2900",
        ]
    );
    assert!(pending_phase_changes(&env, starter.sub_id).await.is_empty());
}

async fn set_term(
    env: &TestEnv,
    sub_id: SubscriptionId,
    term_end: NaiveDate,
    auto_renew: bool,
    uplift: Option<Decimal>,
    notice_days: u32,
) -> meteroid_store::StoreResult<meteroid_store::domain::subscription_phases::SubscriptionTerm> {
    env.services()
        .set_subscription_term(
            Actor::System,
            TENANT_ID,
            sub_id,
            meteroid_store::domain::subscription_phases::SubscriptionTermInput {
                term_end,
                auto_renew,
                renewal_months: 2,
                uplift_percent: uplift,
                notice_days,
            },
        )
        .await
}

/// A term not renewing ends the subscription on its last day: billed January to April, completed
/// on May 1; a phase past it is refused.
/// Rules: R-152
#[rstest]
#[tokio::test]
async fn a_term_without_renewal_completes_on_its_end(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    set_term(&env, starter.sub_id, day(5, 1), false, None, 0)
        .await
        .expect("a term to May");
    let late = set_phases(
        &env,
        starter.sub_id,
        vec![phase(
            day(6, 1),
            "late",
            PhaseTerms {
                components: vec![platform_at(&starter, dec!(31.90))],
                ..PhaseTerms::default()
            },
        )],
    )
    .await;
    assert!(format!("{:?}", late.expect_err("past the term")).contains("term ends"));
    env.run_until(day(6, 2)).await;
    let lines = documents(&env, starter.sub_id).await;
    assert_eq!(lines.len(), 8, "{lines:#?}");
    assert_eq!(
        env.get_subscription(starter.sub_id).await.status,
        diesel_models::enums::SubscriptionStatusEnum::Completed
    );
}

/// Renewing every two months with 10 % uplift: 31.90 and 11.00 from March, then 35.09 and 12.10
/// from May, the term rolling on with each renewal.
/// Rules: R-152, R-146
#[rstest]
#[tokio::test]
async fn an_uplift_renewal_rolls_the_term_on(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let term = set_term(&env, starter.sub_id, day(3, 1), true, Some(dec!(10)), 0)
        .await
        .expect("renewing from March");
    assert!(term.renewal_phase_id.is_some());
    env.run_until(day(5, 2)).await;
    assert_eq!(
        documents(&env, starter.sub_id).await[4..],
        [
            "Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 3190",
            "Recurring 2024-03-01: Seats 2024-03-01..2024-04-01 1100",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 3190",
            "Recurring 2024-04-01: Seats 2024-04-01..2024-05-01 1100",
            "Recurring 2024-05-01: Platform Fee 2024-05-01..2024-06-01 3509",
            "Recurring 2024-05-01: Seats 2024-05-01..2024-06-01 1210",
        ]
    );
    let term = env
        .services()
        .get_subscription_term(TENANT_ID, starter.sub_id)
        .await
        .expect("term")
        .expect("a term");
    assert_eq!(term.term_end, day(7, 1));
    assert_eq!(pending_phase_changes(&env, starter.sub_id).await.len(), 1);
}

/// An opt-out is taken until the notice cutoff, ten days before the renewal; without uplift the
/// term rolls on with nothing written.
/// Rules: R-152
#[rstest]
#[tokio::test]
async fn an_opt_out_waits_for_no_notice_past_its_cutoff(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let term = set_term(&env, starter.sub_id, day(3, 1), true, None, 10)
        .await
        .expect("renewing from March, no uplift");
    assert!(term.renewal_phase_id.is_none());
    assert_eq!(term.notice_cutoff, day(2, 20));

    env.run_until(day(2, 25)).await;
    let late = set_term(&env, starter.sub_id, day(3, 1), false, None, 10).await;
    assert!(format!("{:?}", late.expect_err("past the cutoff")).contains("notice period"));

    env.run_until(day(3, 2)).await;
    let rolled = env
        .services()
        .get_subscription_term(TENANT_ID, starter.sub_id)
        .await
        .expect("term")
        .expect("a term");
    assert_eq!(rolled.term_end, day(5, 1));
    set_term(&env, starter.sub_id, day(5, 1), false, None, 10)
        .await
        .expect("opted out before May's cutoff");
    env.run_until(day(5, 2)).await;
    assert_eq!(
        env.get_subscription(starter.sub_id).await.status,
        diesel_models::enums::SubscriptionStatusEnum::Completed
    );
}

/// Projected from January 15 over six months, the ramp bills 3900 on February 1, 4190 in March
/// and April, then 6509 from May (31807 in all), before anything is invoiced.
/// Rules: R-146
#[rstest]
#[tokio::test]
async fn a_projection_bills_the_phases_ahead(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    set_phases(&env, starter.sub_id, ramp(&starter))
        .await
        .expect("phases set");
    let projection = env
        .services()
        .project_subscription(TENANT_ID, starter.sub_id, 6)
        .await
        .expect("a projection");
    assert_eq!(
        projection
            .invoices
            .iter()
            .map(|invoice| (invoice.on, invoice.total))
            .collect::<Vec<_>>(),
        vec![
            (day(2, 1), 3900),
            (day(3, 1), 4190),
            (day(4, 1), 4190),
            (day(5, 1), 6509),
            (day(6, 1), 6509),
            (day(7, 1), 6509),
        ]
    );
    assert_eq!(projection.total, 31807);
    assert_eq!(projection.invoices[3].lines[0].name, "Platform Fee");
}

fn term_input(
    term_end: NaiveDate,
) -> meteroid_store::domain::subscription_phases::SubscriptionTermInput {
    meteroid_store::domain::subscription_phases::SubscriptionTermInput {
        term_end,
        auto_renew: false,
        renewal_months: 2,
        uplift_percent: None,
        notice_days: 0,
    }
}

/// A term and a ramp set in one batch bill as if set one after the other.
/// Rules: R-156, R-152, R-146
#[rstest]
#[tokio::test]
async fn a_batch_of_timeline_edits_applies_in_order(#[future] test_env: TestEnv) {
    use meteroid_store::domain::subscription_phases::TimelineOp;
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    env.services()
        .apply_subscription_timeline(
            Actor::System,
            TENANT_ID,
            starter.sub_id,
            vec![
                TimelineOp::SetTerm(term_input(day(7, 1))),
                TimelineOp::SetPhases(ramp(&starter)),
            ],
        )
        .await
        .expect("the batch applied");
    assert_eq!(pending_phase_changes(&env, starter.sub_id).await.len(), 2);
    env.run_until(day(7, 2)).await;
    assert_eq!(documents(&env, starter.sub_id).await, RAMP_TO_JUNE);
    assert_eq!(
        env.get_subscription(starter.sub_id).await.status,
        diesel_models::enums::SubscriptionStatusEnum::Completed
    );
}

/// A batch whose later edit is refused leaves nothing of the earlier ones.
/// Rules: R-156
#[rstest]
#[tokio::test]
async fn a_refused_edit_rolls_its_whole_batch_back(#[future] test_env: TestEnv) {
    use meteroid_store::domain::subscription_phases::{DatedDiscountInput, TimelineOp};
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let apply = |ops: Vec<TimelineOp>| {
        env.services()
            .apply_subscription_timeline(Actor::System, TENANT_ID, starter.sub_id, ops)
    };

    let past_term = apply(vec![
        TimelineOp::SetTerm(term_input(day(5, 1))),
        TimelineOp::SetPhases(vec![phase(
            day(6, 1),
            "late",
            PhaseTerms {
                components: vec![platform_at(&starter, dec!(31.90))],
                ..PhaseTerms::default()
            },
        )]),
    ])
    .await;
    assert!(format!("{:?}", past_term.expect_err("past the term")).contains("term ends"));

    let phased_discount = apply(vec![
        TimelineOp::SetPhases(ramp(&starter)),
        TimelineOp::AddDiscount(DatedDiscountInput {
            component: starter.platform,
            percent: dec!(20),
            start_date: day(6, 1),
            until: day(7, 1),
            prorated: false,
            label: None,
            reason: None,
        }),
    ])
    .await;
    assert!(format!("{:?}", phased_discount.expect_err("phased")).contains("still to come"));

    assert!(
        env.services()
            .get_subscription_term(TENANT_ID, starter.sub_id)
            .await
            .expect("term")
            .is_none()
    );
    assert!(pending_phase_changes(&env, starter.sub_id).await.is_empty());
    assert!(
        set_phases(&env, starter.sub_id, vec![])
            .await
            .expect("no phases")
            .is_empty()
    );
}

/// A phase moving to the plan's annual option bills its annual price from the renewal it starts
/// on, and the subscription holds that option once it applied.
/// Rules: R-150
#[rstest]
#[tokio::test]
async fn a_phase_switches_to_another_option_of_its_plan(#[future] test_env: TestEnv) {
    use super::currency_portal::{plan, rate};
    use common_domain::ids::{BaseId, PriceId};
    use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let pro = plan(
        &env,
        "Pro",
        1,
        "EUR",
        vec![
            rate(PriceId::new(), DieselBillingPeriodEnum::Monthly, dec!(20)),
            rate(PriceId::new(), DieselBillingPeriodEnum::Annual, dec!(200)),
        ],
    )
    .await;
    let sub = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(pro)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    set_phases(
        &env,
        sub,
        vec![phase(
            day(3, 1),
            "Annual",
            PhaseTerms {
                plan: Some(meteroid_store::domain::subscription_phases::PhasePlan {
                    plan_version_id: pro,
                    option: Some("annual".to_string()),
                    component_params: vec![],
                    name: None,
                }),
                ..PhaseTerms::default()
            },
        )],
    )
    .await
    .expect("a switch to the annual option");
    env.run_until(day(3, 2)).await;
    assert_eq!(
        env.get_subscription(sub).await.plan_option_id,
        Some(crate::data::plans::option_id(pro, "annual"))
    );
    let lines = documents(&env, sub).await;
    assert!(
        lines
            .iter()
            .any(|line| line.ends_with("2024-03-01..2025-03-01 20000")),
        "{lines:#?}"
    );
}

/// The end date of a subscription with a term is the term's; one before a phase still to start
/// would leave it unapplied: both are refused.
/// Rules: R-146, R-152, R-140
#[rstest]
#[tokio::test]
async fn an_end_date_never_cuts_a_term_or_a_phase(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    set_phases(&env, starter.sub_id, ramp(&starter))
        .await
        .expect("phases set");
    let before_a_phase = env
        .services()
        .change_subscription_end_date(Actor::System, TENANT_ID, starter.sub_id, Some(day(4, 1)))
        .await;
    assert!(format!("{:?}", before_a_phase.expect_err("a phase after it")).contains("2024-05-01"));
    env.services()
        .change_subscription_end_date(Actor::System, TENANT_ID, starter.sub_id, Some(day(7, 1)))
        .await
        .expect("an end after the last phase starts");

    let other = starter_from_jan_1(&env).await;
    set_term(&env, other.sub_id, day(5, 1), false, None, 0)
        .await
        .expect("a term to May");
    let under_a_term = env
        .services()
        .change_subscription_end_date(Actor::System, TENANT_ID, other.sub_id, Some(day(6, 1)))
        .await;
    assert!(format!("{:?}", under_a_term.expect_err("the term's")).contains("contract term"));
}

/// A term renewing with uplift on the day an end-of-period amendment applies renews all the same:
/// the next renewal is written once the step's changes are recorded.
/// Rules: R-152, R-156
#[rstest]
#[tokio::test]
async fn an_uplift_renewal_renews_beside_an_amendment_of_its_day(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    set_term(&env, starter.sub_id, day(3, 1), true, Some(dec!(10)), 0)
        .await
        .expect("renewing from March");
    env.run_until(day(2, 15)).await;
    let sso = super::amendment::create_rate_addon(&env, "SSO", 1500).await;
    env.services()
        .schedule_amendment(
            Actor::System,
            starter.sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::EndOfPeriod,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![meteroid_store::domain::CreateSubscriptionAddOn {
                        add_on_id: sso,
                        customization: meteroid_store::domain::SubscriptionAddOnCustomization::None,
                        quantity: 1,
                    }],
                    edited: vec![],
                    removed: vec![],
                },
            },
        )
        .await
        .expect("SSO from March");
    env.run_until(day(3, 2)).await;
    let term = env
        .services()
        .get_subscription_term(TENANT_ID, starter.sub_id)
        .await
        .expect("term")
        .expect("a term");
    assert_eq!(term.term_end, day(5, 1), "the term rolled on");
    assert!(
        term.renewal_phase_id.is_some(),
        "its next renewal is written"
    );
    let march: Vec<String> = documents(&env, starter.sub_id)
        .await
        .into_iter()
        .filter(|line| line.contains("2024-03-01:"))
        .collect();
    assert!(
        march
            .iter()
            .any(|line| line.contains("Platform Fee") && line.ends_with(" 3190")),
        "{march:#?}"
    );
    assert!(march.iter().any(|line| line.contains("SSO")), "{march:#?}");
}

/// Phases and discounts price a fee from its price: a priced row stored without its fee (as a
/// release that stops writing it will) floors and discounts as any other.
/// Rules: R-146, R-149
#[rstest]
#[tokio::test]
async fn a_priced_fee_without_its_stored_fee_takes_a_floor_and_a_discount(
    #[future] test_env: TestEnv,
) {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::subscription_component::dsl as c;

    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let starter = starter_from_jan_1(&env).await;
    let blanked = diesel::update(
        c::subscription_component
            .filter(c::subscription_id.eq(starter.sub_id))
            .filter(c::price_id.is_not_null()),
    )
    .set(c::legacy_fee.eq(None::<serde_json::Value>))
    .execute(&mut env.conn().await)
    .await
    .expect("the stored fees blanked");
    assert_eq!(blanked, 2, "both fees are priced");

    set_phases(
        &env,
        starter.sub_id,
        vec![phase(
            day(3, 1),
            "floor 3",
            PhaseTerms {
                min_seats: vec![PhaseSeats {
                    quantity_id: starter.seats_quantity,
                    code: String::new(),
                    slots: 3,
                }],
                ..PhaseTerms::default()
            },
        )],
    )
    .await
    .expect("a seat floor");
    discount(
        &env,
        starter.sub_id,
        starter.platform,
        dec!(20),
        day(3, 1),
        day(4, 1),
    )
    .await
    .expect("20 % off in March");

    env.run_until(day(4, 2)).await;
    assert_eq!(
        documents(&env, starter.sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 1000",
            "Recurring 2024-03-01: Platform Fee 2024-03-01..2024-04-01 2320",
            "Recurring 2024-03-01: Seats 2024-03-01..2024-04-01 3000",
            "Recurring 2024-04-01: Platform Fee 2024-04-01..2024-05-01 2900",
            "Recurring 2024-04-01: Seats 2024-04-01..2024-05-01 3000",
        ]
    );
}
