//! The derived state written from the facts: a change scheduled for a period's end is its future
//! rows from the moment it is requested, and the controller applies it in the step that writes
//! the renewal billing it. Each test reads the state and the documents at every step, and I27 at
//! every commit between the boundary and the renewal.
//!
//! Starter is a Platform Fee of 29.00 and Seats at 10.00 (one seat), Pro 99.00 and 25.00: both
//! monthly in advance, from Jan 1, 2024.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId, SubscriptionId,
};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use diesel_models::enums::{BillingPeriodEnum as DieselBillingPeriodEnum, SubscriptionStatusEnum};
use diesel_models::subscription_facts::{SubscriptionPlanWindowRow, SubscriptionTrialRow};
use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum};
use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::subscription_amendment::{
    AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
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

async fn schedule(env: &TestEnv, sub_id: SubscriptionId, to: PlanVersionId) {
    env.services()
        .schedule_plan_change(Actor::System, sub_id, TENANT_ID, to, vec![])
        .await
        .expect("a change at the period end");
}

/// Every invoice line and credit note line of the subscription, in order.
pub(super) async fn documents(env: &TestEnv, sub_id: SubscriptionId) -> Vec<String> {
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

/// Every billing invariant holds now (I27 among them), as at the end of the test.
pub(super) async fn assert_invariants(env: &TestEnv, when: &str) {
    let violations =
        crate::helpers::invariants::credit_invariant_violations(&mut env.conn().await, TENANT_ID)
            .await;
    assert!(violations.is_empty(), "{when}: {violations:#?}");
}

/// Each fee the facts put on a written document is billed exactly once across them.
pub(super) async fn assert_billed_once(env: &TestEnv, sub_id: SubscriptionId) {
    let invoices = env.get_invoices(sub_id).await;
    let owed = crate::helpers::facts::owed_on_written_documents(
        &mut env.conn().await,
        TENANT_ID,
        sub_id,
        &invoices,
        false,
    )
    .await;
    assert_eq!(owed, vec![], "billed exactly once");
}

/// The subscription's MRR movements, as (type, change, day).
async fn movements(env: &TestEnv, sub_id: SubscriptionId) -> Vec<(String, i64, NaiveDate)> {
    use diesel_models::schema::bi_mrr_movement_log::dsl as m;
    m::bi_mrr_movement_log
        .filter(m::subscription_id.eq(sub_id))
        .order_by((m::applies_to.asc(), m::created_at.asc()))
        .select((m::movement_type, m::net_mrr_change, m::applies_to))
        .load::<(diesel_models::enums::MrrMovementType, i64, NaiveDate)>(&mut env.conn().await)
        .await
        .expect("movements")
        .into_iter()
        .map(|(kind, change, on)| (format!("{kind:?}"), change, on))
        .collect()
}

async fn next_wake_at(env: &TestEnv, sub_id: SubscriptionId) -> Option<DateTime<Utc>> {
    env.get_subscription(sub_id).await.next_wake_at
}

/// The renewal of [Jan 1, Feb 1) on a scheduled Starter to Pro change. Read after a fact edit
/// on the boundary day, before the controller ran, nothing is written: the renewal is due, and a
/// document is the controller's alone, so the stored plan stays Starter (I27) and the controller
/// is woken. Its step then applies Pro and bills Pro's February in one transaction.
/// Rules: R-084, R-011
#[rstest]
#[tokio::test]
async fn a_change_at_the_period_end_applies_in_the_step_writing_its_renewal(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = starter_from_jan_1(&env).await;
    schedule(&env, sub_id, PLAN_VERSION_PRO_ID).await;
    assert_eq!(
        next_wake_at(&env, sub_id).await,
        Some(at(2, 1, 0)),
        "the facts change on Feb 1"
    );
    assert_invariants(&env, "scheduled").await;

    // Feb 1, 06:00: the boundary has passed, the controller has not run yet.
    env.set_now(at(2, 1, 6));
    let step = env
        .services()
        .derive_subscription_state(TENANT_ID, sub_id, at(2, 1, 6))
        .await
        .expect("derive")
        .expect("a live subscription");
    assert_eq!(step.transitions.len(), 1, "the plan transition: {step:?}");
    assert_eq!(
        step.documents
            .iter()
            .map(|due| format!("{:?}", due.slot))
            .collect::<Vec<_>>(),
        vec!["Recurring { period_start: 2024-02-01 }"]
    );
    let held = env.get_subscription(sub_id).await;
    assert_eq!(held.plan_version_id, PLAN_VERSION_STARTER_ID);
    held.assert()
        .has_period_start(day(1, 1))
        .has_period_end(day(2, 1));
    assert!(
        held.next_wake_at
            .is_some_and(|wake| wake <= common_clock::now()),
        "the controller is woken at once"
    );
    assert_eq!(env.get_invoices(sub_id).await.len(), 1, "not renewed yet");
    assert_invariants(&env, "after midnight, before the renewal").await;

    env.set_now(at(2, 1, 13));
    env.process_cycles().await;
    let renewed = env.get_subscription(sub_id).await;
    assert_eq!(renewed.plan_version_id, PLAN_VERSION_PRO_ID);
    renewed
        .assert()
        .has_period_start(day(2, 1))
        .has_period_end(day(3, 1));
    assert_invariants(&env, "renewed").await;
    assert_eq!(next_wake_at(&env, sub_id).await, Some(at(3, 1, 0)));
    // Jan 1: Starter's 2900 + 1000. Feb 1: Pro's 9900 + 1 seat × 2500, whole months.
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 9900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 2500",
        ]
    );
    assert_billed_once(&env, sub_id).await;

    // Run again at the same instant: no transition, the stored state is the derived one.
    let rerun = env
        .services()
        .derive_subscription_state(TENANT_ID, sub_id, at(2, 1, 13))
        .await
        .expect("derive")
        .expect("a live subscription");
    assert!(
        rerun.transitions.is_empty() && rerun.state.is_none(),
        "{rerun:?}"
    );
}

/// A Support fee of 50.00 a month added at the period's end: the MRR moves once, with the
/// renewal, by 5000; deriving again writes no second movement.
/// Rules: R-011
#[rstest]
#[tokio::test]
async fn a_scheduled_amendment_moves_the_mrr_once_with_its_renewal(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = starter_from_jan_1(&env).await;
    let before = env.get_subscription(sub_id).await.mrr_cents;
    env.services()
        .schedule_amendment(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::EndOfPeriod,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![AddExtraComponent {
                        name: "Support".to_string(),
                        product: ProductRef::New {
                            name: "Support Product".to_string(),
                            fee_type: FeeTypeEnum::Rate,
                            fee_structure: FeeStructure::Rate {},
                        },
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_string(),
                            pricing: Pricing::Rate {
                                rate: Decimal::new(5000, 2),
                            },
                            credit_unit: None,
                        }),
                    }],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
        )
        .await
        .expect("an amendment at the period end");
    assert_eq!(env.get_subscription(sub_id).await.mrr_cents, before);
    let booked = movements(&env, sub_id).await;

    env.set_now(at(2, 1, 1));
    env.process_cycles().await;
    assert_eq!(env.get_subscription(sub_id).await.mrr_cents, before + 5000);
    let mut expected = booked.clone();
    expected.push(("Expansion".to_string(), 5000, day(2, 1)));
    assert_eq!(movements(&env, sub_id).await, expected);

    let rerun = env
        .services()
        .derive_subscription_state(TENANT_ID, sub_id, at(2, 1, 1))
        .await
        .expect("derive")
        .expect("a live subscription");
    assert!(
        rerun.transitions.is_empty() && rerun.state.is_none(),
        "{rerun:?}"
    );
    env.process_cycles().await;
    assert_eq!(movements(&env, sub_id).await, expected, "booked once");
    // Feb 1: Starter's 2900 + 1000 and Support's 5000, whole months.
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 1000",
            "Recurring 2024-02-01: Support 2024-02-01..2024-03-01 5000",
        ]
    );
    assert_billed_once(&env, sub_id).await;
}

/// Until its first period's New Business is booked, the stored MRR is not the subscription's:
/// the writer books no Expansion the first invoice would book again as New Business.
#[rstest]
#[tokio::test]
async fn no_expansion_is_booked_before_the_first_new_business(#[future] test_env: TestEnv) {
    use diesel_models::schema::subscription::dsl as s;
    use diesel_models::schema::subscription_event::dsl as e;

    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = starter_from_jan_1(&env).await;
    // Fabricated: the creation's MRR still to book, as before a first invoice.
    let mut conn = env.conn().await;
    diesel::update(e::subscription_event.filter(e::subscription_id.eq(sub_id)))
        .set(e::bi_mrr_movement_log_id.eq(None::<uuid::Uuid>))
        .execute(&mut conn)
        .await
        .unwrap();
    diesel::update(s::subscription.filter(s::id.eq(sub_id)))
        .set(s::mrr_cents.eq(0))
        .execute(&mut conn)
        .await
        .unwrap();
    drop(conn);
    let booked = movements(&env, sub_id).await;

    env.services()
        .derive_subscription_state(TENANT_ID, sub_id, at(1, 15, 12))
        .await
        .expect("derive");
    assert_eq!(env.get_subscription(sub_id).await.mrr_cents, 0);
    assert_eq!(movements(&env, sub_id).await, booked, "no movement");
}

/// R-092: a change scheduled before a cancellation still to come applies on its date, and the
/// cancellation then ends the subscription on Pro, its last period billed from the facts.
/// Rules: R-092, R-016
#[rstest]
#[tokio::test]
async fn a_change_before_a_pending_cancellation_applies_then_the_cancellation_ends_it(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = starter_from_jan_1(&env).await;
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(3, 1)),
        )
        .await
        .expect("cancel on Mar 1");
    schedule(&env, sub_id, PLAN_VERSION_PRO_ID).await;

    env.set_now(at(2, 1, 1));
    env.process_cycles().await;
    assert_eq!(
        env.get_subscription(sub_id).await.plan_version_id,
        PLAN_VERSION_PRO_ID
    );
    env.set_now(at(3, 1, 1));
    env.process_cycles().await;
    env.process_due_events().await;
    let ended = env.get_subscription(sub_id).await;
    ended.assert().has_status(SubscriptionStatusEnum::Cancelled);
    assert_eq!(ended.plan_version_id, PLAN_VERSION_PRO_ID);
    // Jan: Starter; Feb: Pro's 9900 + 2500. The cancellation on Mar 1 bills nothing more: every
    // fee is in advance and ends with February.
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 9900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 2500",
        ]
    );
    assert_billed_once(&env, sub_id).await;
}

/// A paid trial of 45 days from Jan 1 runs to Feb 15; a change at the period's end, Feb 1, ends it
/// there: its trial fact ends on Feb 1 with the change, and the subscription is Active from then on
/// the new plan, billed whole from Feb 1.
#[rstest]
#[tokio::test]
async fn a_change_ending_a_paid_trial_moves_its_trial_end_to_its_date(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let plan = |name: &'static str, cents: i64, trial: bool| {
        let version = PlanVersionId::new();
        let mut seed = PlanSeed::new(PlanId::new(), name, version);
        if trial {
            seed = seed.trial(45, PLAN_ENTERPRISE_ID, false);
        }
        let seed = seed.components(vec![SeedComp::rate(
            PriceComponentId::new(),
            "Monthly Rate",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            Decimal::new(cents, 2),
        )]);
        (version, seed)
    };
    let (trial_plan, seed) = plan("Paid trial at 100", 10_000, true);
    seed.seed(&mut env.conn().await).await.expect("trial plan");
    let (next, seed) = plan("Next at 200", 20_000, false);
    seed.seed(&mut env.conn().await).await.expect("next plan");
    let sub_id = subscription()
        .plan_version(trial_plan)
        .start_date(day(1, 1))
        .on_start()
        .trial_days(45)
        .create(env.services())
        .await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_trial_active();
    let trial_until = || async {
        SubscriptionTrialRow::find(&mut env.conn().await, sub_id)
            .await
            .unwrap()
            .expect("the trial fact")
            .until_date
    };
    assert_eq!(trial_until().await, day(2, 15));

    schedule(&env, sub_id, next).await;
    assert_eq!(trial_until().await, day(2, 1), "moved to the change");

    env.set_now(at(2, 1, 1));
    env.process_cycles().await;
    env.process_due_events().await;
    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_active();
    assert_eq!(sub.plan_version_id, next);
    // Jan 1: the paid trial's month, 10000. Feb 1: the new plan's month, 20000.
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Monthly Rate 2024-01-01..2024-02-01 10000",
            "Recurring 2024-02-01: Monthly Rate 2024-02-01..2024-03-01 20000",
        ]
    );
    assert_billed_once(&env, sub_id).await;
}

/// The plan windows, fee rows and trial of a subscription, as they read.
async fn snapshot(env: &TestEnv, sub_id: SubscriptionId) -> Vec<String> {
    let mut conn = env.conn().await;
    let mut rows: Vec<String> = SubscriptionPlanWindowRow::list_by_subscription(&mut conn, sub_id)
        .await
        .unwrap()
        .into_iter()
        .map(|w| {
            format!(
                "window {} {:?}..{:?} {:?}",
                w.plan_version_id, w.from_date, w.to_date, w.cadence
            )
        })
        .collect();
    rows.extend(
        diesel_models::subscription_components::SubscriptionComponentRow::list_fact_rows(
            &mut conn, sub_id,
        )
        .await
        .unwrap()
        .into_iter()
        .map(|c| {
            format!(
                "component {} {}..{:?}",
                c.name, c.effective_from, c.effective_to
            )
        }),
    );
    rows.extend(
        SubscriptionTrialRow::find(&mut conn, sub_id)
            .await
            .unwrap()
            .map(|t| format!("trial {}..{}", t.from_date, t.until_date)),
    );
    rows.sort();
    rows
}

/// Cancelling a pending change withdraws its rows, bumps the customer and derives the state
/// again: the renewal bills Starter, no switch is recorded.
#[rstest]
#[tokio::test]
async fn cancelling_a_pending_change_withdraws_its_rows_and_derives_again(
    #[future] test_env: TestEnv,
) {
    use diesel_models::schema::billing_controller::dsl as bc;

    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = starter_from_jan_1(&env).await;
    env.process_cycles().await;
    let before = snapshot(&env, sub_id).await;
    schedule(&env, sub_id, PLAN_VERSION_PRO_ID).await;
    env.process_cycles().await;
    let dirty = || async {
        bc::billing_controller
            .filter(bc::customer_id.eq(CUST_UBER_ID))
            .select(bc::dirty_since)
            .first::<Option<DateTime<Utc>>>(&mut env.conn().await)
            .await
            .unwrap()
    };
    assert_eq!(dirty().await, None);

    env.services()
        .cancel_plan_change(Actor::System, sub_id, TENANT_ID)
        .await
        .expect("cancel the pending change");
    assert!(dirty().await.is_some(), "the customer is bumped");
    assert_eq!(snapshot(&env, sub_id).await, before, "its rows withdrawn");
    assert_eq!(
        next_wake_at(&env, sub_id).await,
        Some(at(2, 1, 0)),
        "the renewal's"
    );
    assert_invariants(&env, "cancelled").await;

    env.set_now(at(2, 1, 1));
    env.process_cycles().await;
    assert_eq!(
        env.get_subscription(sub_id).await.plan_version_id,
        PLAN_VERSION_STARTER_ID
    );
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 1000",
        ]
    );
}

/// An immediate change on the boundary day before the controller ran: its rows start on Feb 1 and
/// the renewal is due, a document the controller's alone, so the change's own transaction writes
/// no state: the stored plan stays Starter (I27) until the step that renews applies Pro.
/// Rules: R-084
#[rstest]
#[tokio::test]
async fn an_immediate_change_on_the_boundary_day_shows_the_old_plan_until_the_controller(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = starter_from_jan_1(&env).await;
    env.set_now(at(2, 1, 6));
    env.services()
        .apply_plan_change_immediate(
            Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
        )
        .await
        .expect("an immediate change on Feb 1");
    let held = env.get_subscription(sub_id).await;
    assert_eq!(
        held.plan_version_id, PLAN_VERSION_STARTER_ID,
        "until the renewal"
    );
    held.assert().has_period_end(day(2, 1));
    assert_invariants(&env, "changed, not renewed").await;

    env.process_cycles().await;
    let renewed = env.get_subscription(sub_id).await;
    assert_eq!(renewed.plan_version_id, PLAN_VERSION_PRO_ID);
    renewed
        .assert()
        .has_period_start(day(2, 1))
        .has_period_end(day(3, 1));
    assert_invariants(&env, "renewed").await;
    // January was used whole: nothing given back, no stub. Feb 1 bills Pro's month.
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 9900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 2500",
        ]
    );
    assert_billed_once(&env, sub_id).await;
}

/// The blind spot: each charge the facts put on a document already written (January's invoice,
/// written before the change) is billed exactly once across every document and note, for each
/// immediate change of January 15. The change's adjustment and notes settle what the written
/// invoice keeps (R-077), and the writer applies the plan, its period and the MRR.
/// Rules: R-077, R-079, R-080
#[rstest]
#[case::plan_change(Change::Plan)]
#[case::amendment(Change::Amendment)]
#[case::seats(Change::Seats)]
#[case::cancellation(Change::Cancellation)]
#[tokio::test]
async fn each_charge_on_a_written_document_is_billed_once(
    #[future] test_env: TestEnv,
    #[case] change: Change,
) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = starter_from_jan_1(&env).await;
    match change {
        Change::Plan => {
            env.services()
                .apply_plan_change_immediate(
                    Actor::System,
                    sub_id,
                    TENANT_ID,
                    PLAN_VERSION_PRO_ID,
                    vec![],
                )
                .await
                .expect("Pro from Jan 15");
        }
        Change::Amendment => {
            env.services()
                .apply_amendment_immediate(
                    Actor::System,
                    sub_id,
                    TENANT_ID,
                    SubscriptionAmendment {
                        apply_mode: PlanChangeMode::Immediate,
                        component_changes: ComponentChanges {
                            edited: vec![],
                            added: vec![AddExtraComponent {
                                name: "Support".to_string(),
                                product: ProductRef::New {
                                    name: "Support Product".to_string(),
                                    fee_type: FeeTypeEnum::Rate,
                                    fee_structure: FeeStructure::Rate {},
                                },
                                price_entry: PriceEntry::New(PriceInput {
                                    cadence: BillingPeriodEnum::Monthly,
                                    currency: "EUR".to_string(),
                                    pricing: Pricing::Rate {
                                        rate: Decimal::new(3100, 2),
                                    },
                                    credit_unit: None,
                                }),
                            }],
                            removed: vec![],
                        },
                        add_on_changes: AddOnChanges::default(),
                    },
                )
                .await
                .expect("Support from Jan 15");
        }
        Change::Seats => {
            env.services()
                .update_subscription_slots(
                    TENANT_ID,
                    sub_id,
                    COMP_STARTER_SEATS_ID,
                    2,
                    meteroid_store::domain::SlotUpgradeBillingMode::Optimistic,
                    Actor::System,
                )
                .await
                .expect("two more seats from Jan 15");
        }
        Change::Cancellation => {
            env.services()
                .cancel_subscription(
                    Actor::System,
                    sub_id,
                    TENANT_ID,
                    None,
                    CancellationEffectiveAt::Date(day(1, 15)),
                )
                .await
                .expect("cancelled on Jan 15");
            env.process_cycles().await;
        }
    }
    env.set_now(at(2, 1, 1));
    env.process_cycles().await;
    assert_invariants(&env, "changed").await;
    assert_eq!(documents(&env, sub_id).await, change.documents());
    assert_billed_once(&env, sub_id).await;
}

#[derive(Clone, Copy, Debug)]
enum Change {
    Plan,
    Amendment,
    Seats,
    Cancellation,
}

impl Change {
    /// January's 31 days: from Jan 15, 17 are left (R-079: what is left at the start less what
    /// is left at the end, each rounded half away from zero).
    fn documents(self) -> Vec<&'static str> {
        match self {
            // Starter gives back 2900 × 17/31 = 1590.32 → 1590 and 1000 × 17/31 = 548.39 → 548;
            // Pro's stubs 9900 × 17/31 = 5429.03 → 5429 and 2500 × 17/31 = 1370.97 → 1371; Feb 1
            // bills Pro's month.
            Change::Plan => vec![
                "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
                "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
                "note: Platform Fee 2024-01-01..2024-02-01 -1590",
                "note: Seats 2024-01-01..2024-02-01 -548",
                "Adjustment 2024-01-15: Platform Fee (prorated) 2024-01-15..2024-02-01 5429",
                "Adjustment 2024-01-15: Seats (prorated) 2024-01-15..2024-02-01 1371",
                "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 9900",
                "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 2500",
            ],
            // Support's stub 3100 × 17/31 = 1700; Feb 1 bills it whole.
            Change::Amendment => vec![
                "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
                "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
                "Adjustment 2024-01-15: Support (prorated) 2024-01-15..2024-02-01 1700",
                "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
                "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 1000",
                "Recurring 2024-02-01: Support 2024-02-01..2024-03-01 3100",
            ],
            // Two seats more for 17 days: 2 × 1000 × 17/31 = 1096.77 → 1097; Feb 1 bills 3 seats.
            Change::Seats => vec![
                "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
                "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
                "OneOff 2024-01-15: 2 Seats upgrade 2024-01-15..2024-02-01 1097",
                "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 2900",
                "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 3000",
            ],
            // The unused 17 days given back: 1590 and 548.
            Change::Cancellation => vec![
                "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
                "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
                "note: Platform Fee 2024-01-01..2024-02-01 -1590",
                "note: Seats 2024-01-01..2024-02-01 -548",
            ],
        }
    }
}

/// Two seats bought on Jan 10, then Pro from Jan 20: the upgrade's seats are given back for the
/// days Pro takes over, as January's seat is. The purchase bills the Seats fee's January charge, so
/// reconcile reads it: each charge billed once across the documents.
/// Rules: R-077, R-079, R-080
#[rstest]
#[tokio::test]
async fn a_change_after_a_seat_purchase_gives_back_the_purchased_seats(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(at(1, 10, 12));
    let sub_id = starter_from_jan_1(&env).await;
    env.services()
        .update_subscription_slots(
            TENANT_ID,
            sub_id,
            COMP_STARTER_SEATS_ID,
            2,
            meteroid_store::domain::SlotUpgradeBillingMode::Optimistic,
            Actor::System,
        )
        .await
        .expect("two more seats from Jan 10");
    env.set_now(at(1, 20, 12));
    env.services()
        .apply_plan_change_immediate(
            Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
        )
        .await
        .expect("Pro from Jan 20");
    env.set_now(at(2, 1, 1));
    env.process_cycles().await;
    // January has 31 days; 22 from Jan 10, 12 from Jan 20 (R-079).
    // Bought: 2 × 1000 × 22/31 = 1419.35 → 1419. From Jan 20, Starter gives back 2900 × 12/31 =
    // 1122.58 → 1123, the seat 1000 × 12/31 = 387.10 → 387, the two bought 2000 × 12/31 =
    // 774.19 → 774; Pro bills 9900 × 12/31 = 3832.26 → 3832 and 3 seats 7500 × 12/31 = 2903.23
    // → 2903. Feb 1: Pro's month, 9900 and 3 × 2500.
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "note: Platform Fee 2024-01-01..2024-02-01 -1123",
            "note: Seats 2024-01-01..2024-02-01 -387",
            "OneOff 2024-01-10: 2 Seats upgrade 2024-01-10..2024-02-01 1419",
            "note: 2 Seats upgrade 2024-01-10..2024-02-01 -774",
            "Adjustment 2024-01-20: Platform Fee (prorated) 2024-01-20..2024-02-01 3832",
            "Adjustment 2024-01-20: Seats (prorated) 2024-01-20..2024-02-01 2903",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 9900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 7500",
        ]
    );
    assert_billed_once(&env, sub_id).await;
}

/// What the writer costs: a customer of 40 subscriptions, each with a change at the period's end,
/// renewed in one controller pass. Each step loads the facts and evaluates them once.
#[rstest]
#[tokio::test]
async fn a_customer_of_many_subscriptions_is_derived_in_one_pass(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let mut subscriptions = Vec::new();
    for _ in 0..40 {
        let sub_id = starter_from_jan_1(&env).await;
        schedule(&env, sub_id, PLAN_VERSION_PRO_ID).await;
        subscriptions.push(sub_id);
    }
    let started = std::time::Instant::now();
    for sub_id in &subscriptions {
        env.services()
            .derive_subscription_state(TENANT_ID, *sub_id, at(1, 15, 12))
            .await
            .expect("derive");
    }
    let per_subscription = started.elapsed() / 40;
    eprintln!("derived state: {per_subscription:?} a subscription");
    assert!(
        per_subscription < std::time::Duration::from_millis(500),
        "{per_subscription:?} a subscription"
    );
    env.set_now(at(2, 1, 1));
    env.process_cycles().await;
    for sub_id in &subscriptions {
        assert_eq!(
            env.get_subscription(*sub_id).await.plan_version_id,
            PLAN_VERSION_PRO_ID
        );
    }
}

/// Two seats bought Jan 10 by the previous release, its line naming no fee row: backfilled, it
/// bills as this release's lines. A seat on Jan 15 bills 1000 × 92/31 → 2968 − 2419 = 549; ended Jan
/// 20, each line gives back 12 days: 1123, 549 × 12/17 → 388, 387, and the Jan 10 seats 773.
/// Rules: R-077, R-079, R-080
#[rstest]
#[tokio::test]
async fn a_seat_purchase_the_previous_release_invoiced_is_given_back_as_the_seats_fee(
    #[future] test_env: TestEnv,
) {
    use diesel_async::SimpleAsyncConnection;

    let env = test_env.await;
    env.set_now(at(1, 10, 12));
    let sub_id = starter_from_jan_1(&env).await;
    let buy = |seats: i32| {
        env.services().update_subscription_slots(
            TENANT_ID,
            sub_id,
            COMP_STARTER_SEATS_ID,
            seats,
            meteroid_store::domain::SlotUpgradeBillingMode::Optimistic,
            Actor::System,
        )
    };
    buy(2).await.expect("two more seats from Jan 10");
    // The line as the previous release wrote it.
    env.conn()
        .await
        .batch_execute(&format!(
            "UPDATE invoice SET line_items = (
                 SELECT jsonb_agg(CASE WHEN l ->> 'name' LIKE '%Seats upgrade'
                                       THEN l || '{{\"sub_component_id\": null}}'::jsonb
                                       ELSE l END ORDER BY ord)
                 FROM jsonb_array_elements(line_items) WITH ORDINALITY AS e(l, ord))
             WHERE subscription_id = '{sub_id}'",
            sub_id = sub_id.as_uuid()
        ))
        .await
        .expect("the line in its old shape");
    let backfill = crate::release_migrations::migration_section(
        crate::release_migrations::BILLING_ON_FACTS_UP,
        "Seat upgrade lines",
    )
    .replace("CURRENT_DATE", "'2024-01-12'::date");
    env.conn()
        .await
        .batch_execute(&backfill)
        .await
        .expect("the backfill runs");

    env.set_now(at(1, 15, 12));
    buy(1).await.expect("one more seat from Jan 15");
    env.set_now(at(1, 20, 0));
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await
        .expect("ended on Jan 20");
    env.set_now(at(2, 1, 1));
    env.process_cycles().await;
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "note: Platform Fee 2024-01-01..2024-02-01 -1123",
            "note: Seats 2024-01-01..2024-02-01 -387",
            "OneOff 2024-01-10: 2 Seats upgrade 2024-01-10..2024-02-01 1419",
            "note: 2 Seats upgrade 2024-01-10..2024-02-01 -773",
            "OneOff 2024-01-15: 1 Seats upgrade 2024-01-15..2024-02-01 549",
            "note: 1 Seats upgrade 2024-01-15..2024-02-01 -388",
        ]
    );
    assert_billed_once(&env, sub_id).await;
}
