//! Pending scheduled changes written as facts. A change the previous release
//! left pending (here: one written now, then the rollback script run over it) resolves against
//! its target version, an amendment as it is; the writer writes them once and marks the event
//! migrated, at the deploy or when the event comes due, never applying it.
use std::collections::HashSet;
use std::fmt::Debug;
use std::hash::Hash;

use chrono::{NaiveDate, NaiveTime};
use common_domain::actor::Actor;
use common_domain::ids::{
    AddOnId, BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId, ProductId,
    SubscriptionAddOnId, SubscriptionId, SubscriptionPriceComponentId,
};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use diesel_models::enums::{
    ActorTypeEnum, BillingPeriodEnum as DieselBillingPeriodEnum,
    ScheduledEventStatus as DieselEventStatus, SubscriptionStatusEnum as DieselStatus,
};
use diesel_models::scheduled_events::ScheduledEventRow;
use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use diesel_models::subscription_facts::{SubscriptionEndingRow, SubscriptionPlanWindowRow};
use rstest::rstest;
use rust_decimal::Decimal;

use super::amendment::create_rate_addon;
use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env};
use meteroid_store::domain::SubscriptionStatusEnum;
use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum, ScheduledEventStatus};
use meteroid_store::domain::plan_version_add_ons::PlanVersionAddOnNew;
use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::scheduled_events::ScheduledEvent;
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
};
use meteroid_store::domain::subscription_amendment::{
    AddExtraComponent, AddOnChanges, ComponentChanges, EditComponent, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::repositories::plan_version_add_ons::PlanVersionAddOnInterface;
use meteroid_store::services::facts::pending::{Migration, PendingFacts, Unresolvable};

fn day(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, month, day).unwrap()
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

/// The rollback script of the release, run over every change written since the epoch.
pub(crate) async fn roll_back_changes_as_facts(env: &TestEnv) {
    use diesel_async::SimpleAsyncConnection;

    let script = include_str!("../../../migrations/rollback/changes-as-facts.sql")
        .replace(":deployed_at", "'1970-01-01'");
    env.conn()
        .await
        .batch_execute(&script)
        .await
        .expect("the rollback script runs");
}

async fn reload(env: &TestEnv, event: &ScheduledEvent) -> ScheduledEvent {
    let mut conn = env.conn().await;
    ScheduledEventRow::get_by_id(&mut conn, event.id, &TENANT_ID)
        .await
        .expect("the event")
        .try_into()
        .expect("a scheduled event")
}

/// A change to `to` at the period's end, Feb 1, as the previous release left it: pending, no
/// row written.
async fn schedule(env: &TestEnv, sub_id: SubscriptionId, to: PlanVersionId) -> ScheduledEvent {
    let event = env
        .services()
        .schedule_plan_change(Actor::System, sub_id, TENANT_ID, to, vec![])
        .await
        .expect("a change at the period end");
    roll_back_changes_as_facts(env).await;
    reload(env, &event).await
}

async fn resolved(env: &TestEnv, event: &ScheduledEvent) -> PendingFacts {
    env.services()
        .resolve_pending_event(TENANT_ID, event.id)
        .await
        .expect("resolve")
        .expect("the pending change resolves")
}

/// A fee row without its id.
#[derive(Debug, PartialEq)]
struct Fee {
    price_component_id: Option<PriceComponentId>,
    product_id: Option<ProductId>,
    price_id: Option<PriceId>,
    name: String,
    period: String,
    fee: Option<serde_json::Value>,
    effective_from: NaiveDate,
    lineage_id: Option<SubscriptionPriceComponentId>,
    added_by_amendment: bool,
}

fn opened_component(row: &SubscriptionComponentRowNew) -> Fee {
    Fee {
        price_component_id: row.price_component_id,
        product_id: row.product_id,
        price_id: row.price_id,
        name: row.name.clone(),
        period: format!("{:?}", row.period),
        fee: row.legacy_fee.clone(),
        effective_from: row.effective_from,
        lineage_id: row.lineage_id,
        added_by_amendment: row.added_by_amendment,
    }
}

fn stored_component(row: &SubscriptionComponentRow) -> Fee {
    Fee {
        price_component_id: row.price_component_id,
        product_id: row.product_id,
        price_id: row.price_id,
        name: row.name.clone(),
        period: format!("{:?}", row.period),
        fee: row.legacy_fee.clone(),
        effective_from: row.effective_from,
        lineage_id: row.lineage_id,
        added_by_amendment: row.added_by_amendment,
    }
}

/// An add-on row without its id.
#[derive(Debug, PartialEq)]
struct AddOnFee {
    add_on_id: AddOnId,
    product_id: Option<ProductId>,
    price_id: Option<PriceId>,
    name: String,
    period: String,
    fee: Option<serde_json::Value>,
    quantity: i32,
    effective_from: NaiveDate,
    lineage_id: Option<SubscriptionAddOnId>,
    added_by_amendment: bool,
}

fn opened_add_on(row: &SubscriptionAddOnRowNew) -> AddOnFee {
    AddOnFee {
        add_on_id: row.add_on_id,
        product_id: row.product_id,
        price_id: row.price_id,
        name: row.name.clone(),
        period: format!("{:?}", row.period),
        fee: row.legacy_fee.clone(),
        quantity: row.quantity,
        effective_from: row.effective_from,
        lineage_id: row.lineage_id,
        added_by_amendment: row.added_by_amendment,
    }
}

fn stored_add_on(row: &SubscriptionAddOnRow) -> AddOnFee {
    AddOnFee {
        add_on_id: row.add_on_id,
        product_id: row.product_id,
        price_id: row.price_id,
        name: row.name.clone(),
        period: format!("{:?}", row.period),
        fee: row.legacy_fee.clone(),
        quantity: row.quantity,
        effective_from: row.effective_from,
        lineage_id: row.lineage_id,
        added_by_amendment: row.added_by_amendment,
    }
}

fn sorted<T: Debug>(rows: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut rows: Vec<T> = rows.into_iter().collect();
    rows.sort_by_key(|row| format!("{row:?}"));
    rows
}

fn set<T: Eq + Hash + Copy>(ids: &[T]) -> HashSet<T> {
    ids.iter().copied().collect()
}

/// What the subscription's rows hold on `on`: the rows closed then, the rows opened then, and
/// the plan window from then.
struct Written {
    closed_components: Vec<SubscriptionPriceComponentId>,
    opened_components: Vec<Fee>,
    closed_add_ons: Vec<SubscriptionAddOnId>,
    opened_add_ons: Vec<AddOnFee>,
    window: Option<SubscriptionPlanWindowRow>,
}

async fn written_on(env: &TestEnv, sub_id: SubscriptionId, on: NaiveDate) -> Written {
    let mut conn = env.conn().await;
    let components = SubscriptionComponentRow::list_fact_rows(&mut conn, sub_id)
        .await
        .expect("component rows");
    let add_ons = SubscriptionAddOnRow::list_fact_rows(&mut conn, sub_id)
        .await
        .expect("add-on rows");
    let window = SubscriptionPlanWindowRow::list_by_subscription(&mut conn, sub_id)
        .await
        .expect("plan windows")
        .into_iter()
        .find(|window| window.from_date == on);
    Written {
        closed_components: components
            .iter()
            .filter(|row| row.effective_to == Some(on))
            .map(|row| row.id)
            .collect(),
        opened_components: components
            .iter()
            .filter(|row| row.effective_from == on)
            .map(stored_component)
            .collect(),
        closed_add_ons: add_ons
            .iter()
            .filter(|row| row.effective_to == Some(on))
            .map(|row| row.id)
            .collect(),
        opened_add_ons: add_ons
            .iter()
            .filter(|row| row.effective_from == on)
            .map(stored_add_on)
            .collect(),
        window,
    }
}

/// The rows on the change's day are the resolved ones: closed, opened, the plan window.
async fn assert_written_as_resolved(env: &TestEnv, facts: &PendingFacts) {
    let written = written_on(env, facts.subscription_id, facts.on).await;
    assert_eq!(
        set(&written.closed_components),
        set(&facts.closes_components),
        "component rows closed on {}",
        facts.on
    );
    assert_eq!(
        sorted(written.opened_components),
        sorted(facts.opens_components.iter().map(opened_component)),
        "component rows opened on {}",
        facts.on
    );
    assert_eq!(
        set(&written.closed_add_ons),
        set(&facts.closes_add_ons),
        "add-on rows closed on {}",
        facts.on
    );
    assert_eq!(
        sorted(written.opened_add_ons),
        sorted(facts.opens_add_ons.iter().map(opened_add_on)),
        "add-on rows opened on {}",
        facts.on
    );
    match (&facts.plan_window, &written.window) {
        (Some(resolved), Some(window)) => {
            assert_eq!(window.plan_version_id, resolved.plan_version_id);
            assert_eq!(window.from_date, resolved.from_date);
            assert_eq!(
                BillingPeriodEnum::from(window.cadence.clone()),
                resolved.cadence
            );
            assert_eq!(window.cadence_origin, resolved.cadence_origin);
            assert_eq!(window.to_date, None, "the change's window stays open");
        }
        (None, None) => {}
        (resolved, window) => panic!("plan window resolved {resolved:?}, written {window:?}"),
    }
}

/// Starter to Pro at Feb 1, monthly both: both fees close and reopen on Pro's prices, on the
/// grid in force.
#[rstest]
#[tokio::test]
async fn a_scheduled_plan_change_resolves_to_the_rows_it_writes(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let sub_id = starter_from_jan_1(&env).await;
    let event = schedule(&env, sub_id, PLAN_VERSION_PRO_ID).await;

    let facts = resolved(&env, &event).await;
    assert_eq!(facts.on, day(2, 1));
    assert_eq!(facts.closes_components.len(), 2);
    let prices: HashSet<Option<PriceId>> = facts
        .opens_components
        .iter()
        .map(|row| row.price_id)
        .collect();
    assert_eq!(
        prices,
        HashSet::from([Some(PRICE_PRO_PLATFORM_FEE_ID), Some(PRICE_PRO_SEATS_ID)])
    );
    let window = facts.plan_window.as_ref().expect("a plan window");
    assert_eq!(window.plan_version_id, PLAN_VERSION_PRO_ID);
    assert_eq!(window.cadence, BillingPeriodEnum::Monthly);
    assert_eq!(
        window.cadence_origin,
        day(1, 1),
        "the monthly grid keeps its origin"
    );
    assert!(
        written_on(&env, sub_id, day(2, 1))
            .await
            .opened_components
            .is_empty()
    );

    env.process_cycles().await;
    assert_eq!(
        env.get_subscription(sub_id).await.plan_version_id,
        PLAN_VERSION_PRO_ID,
        "applied on Feb 1"
    );
    assert_written_as_resolved(&env, &facts).await;
}

/// A plan of one Platform Fee of `fee` a period, in advance, on `period`.
async fn platform_fee_plan(
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

/// Monthly to quarterly at Feb 1: the new window runs quarterly from its own origin (R-084).
#[rstest]
#[tokio::test]
async fn a_scheduled_change_of_cadence_resolves_to_the_rows_it_writes(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let monthly = platform_fee_plan(
        &env,
        "Monthly",
        DieselBillingPeriodEnum::Monthly,
        Decimal::new(29, 0),
    )
    .await;
    let quarterly = platform_fee_plan(
        &env,
        "Quarterly",
        DieselBillingPeriodEnum::Quarterly,
        Decimal::new(300, 0),
    )
    .await;
    let sub_id = subscription()
        .plan_version(monthly)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let event = schedule(&env, sub_id, quarterly).await;

    let facts = resolved(&env, &event).await;
    let window = facts.plan_window.as_ref().expect("a plan window");
    assert_eq!(window.cadence, BillingPeriodEnum::Quarterly);
    assert_eq!(window.from_date, day(2, 1));

    env.process_cycles().await;
    assert_eq!(
        env.get_subscription(sub_id).await.plan_version_id,
        quarterly
    );
    assert_written_as_resolved(&env, &facts).await;
}

/// Two add-ons on Starter, Pro offering one: the other ends with the old plan on Feb 1.
#[rstest]
#[tokio::test]
async fn a_scheduled_change_ends_the_add_ons_its_target_does_not_offer(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let dropped = create_rate_addon(&env, "Premium Support", 2000).await;
    let kept = create_rate_addon(&env, "Priority Line", 1000).await;
    env.store()
        .attach_add_on_to_plan_version(PlanVersionAddOnNew {
            plan_version_id: PLAN_VERSION_PRO_ID,
            add_on_id: kept,
            price_id: None,
            self_serviceable: None,
            max_instances_per_subscription: None,
            tenant_id: TENANT_ID,
        })
        .await
        .expect("Pro offers the kept add-on");
    let sub_id = starter_from_jan_1(&env).await;
    let add = |add_on_id| CreateSubscriptionAddOn {
        add_on_id,
        customization: SubscriptionAddOnCustomization::None,
        quantity: 1,
    };
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges::default(),
                add_on_changes: AddOnChanges {
                    added: vec![add(dropped), add(kept)],
                    edited: vec![],
                    removed: vec![],
                },
            },
            day(1, 1),
        )
        .await
        .expect("both add-ons from Jan 1");
    let event = schedule(&env, sub_id, PLAN_VERSION_PRO_ID).await;

    let facts = resolved(&env, &event).await;
    let mut conn = env.conn().await;
    let dropped_row = SubscriptionAddOnRow::list_fact_rows(&mut conn, sub_id)
        .await
        .expect("add-on rows")
        .into_iter()
        .find(|row| row.add_on_id == dropped)
        .expect("the dropped add-on's row")
        .id;
    drop(conn);
    assert_eq!(facts.closes_add_ons, vec![dropped_row]);
    assert!(facts.opens_add_ons.is_empty());

    env.process_cycles().await;
    assert_written_as_resolved(&env, &facts).await;
}

/// An end-of-period amendment on Starter: Platform Fee repriced, a Support fee and an add-on
/// added. Written as it is: the repriced fee continues its lineage.
#[rstest]
#[tokio::test]
async fn a_scheduled_amendment_resolves_to_the_rows_it_writes(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let add_on = create_rate_addon(&env, "Premium Support", 2000).await;
    let sub_id = starter_from_jan_1(&env).await;
    let platform = env
        .get_subscription_components(sub_id)
        .await
        .into_iter()
        .find(|row| row.price_component_id == Some(COMP_STARTER_PLATFORM_FEE_ID))
        .expect("Starter's Platform Fee");
    let monthly_rate = |cents| {
        PriceEntry::New(PriceInput {
            cadence: BillingPeriodEnum::Monthly,
            currency: "EUR".to_string(),
            pricing: Pricing::Rate {
                rate: Decimal::new(cents, 2),
            },
            credit_unit: None,
        })
    };
    let event = env
        .services()
        .schedule_amendment(
            Actor::System,
            sub_id,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::EndOfPeriod,
                component_changes: ComponentChanges {
                    edited: vec![EditComponent {
                        subscription_component_id: platform.id,
                        name: None,
                        price_entry: monthly_rate(3900),
                    }],
                    added: vec![AddExtraComponent {
                        name: "Support".to_string(),
                        product: ProductRef::New {
                            name: "Support Product".to_string(),
                            fee_type: FeeTypeEnum::Rate,
                            fee_structure: FeeStructure::Rate {},
                        },
                        price_entry: monthly_rate(5000),
                    }],
                    removed: vec![],
                },
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
        )
        .await
        .expect("an amendment at the period end");
    roll_back_changes_as_facts(&env).await;
    let event = reload(&env, &event).await;

    let facts = resolved(&env, &event).await;
    assert_eq!(facts.on, day(2, 1));
    assert!(facts.plan_window.is_none(), "an amendment keeps the plan");
    assert_eq!(facts.closes_components, vec![platform.id]);
    assert_eq!(facts.opens_components.len(), 2);
    assert!(
        facts
            .opens_components
            .iter()
            .any(|row| row.lineage_id == Some(platform.lineage_id.unwrap_or(platform.id))),
        "the repriced fee continues the Platform Fee's lineage"
    );
    assert_eq!(facts.opens_add_ons.len(), 1);

    env.process_cycles().await;
    assert_written_as_resolved(&env, &facts).await;
}

/// The writer writes the resolved rows and marks the event migrated; run again, it writes
/// nothing more.
#[rstest]
#[tokio::test]
async fn a_pending_change_is_written_as_facts_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let sub_id = starter_from_jan_1(&env).await;
    let event = schedule(&env, sub_id, PLAN_VERSION_PRO_ID).await;
    let facts = resolved(&env, &event).await;

    let first = env
        .services()
        .write_pending_event_as_facts(TENANT_ID, event.id)
        .await
        .expect("write")
        .expect("the pending change resolves");
    let Migration::Written(written) = first else {
        panic!("the first run writes");
    };
    assert_written_as_resolved(&env, &written).await;
    assert_eq!(
        sorted(written.opens_components.iter().map(opened_component)),
        sorted(facts.opens_components.iter().map(opened_component)),
        "the writer writes what the dry run resolved"
    );

    let second = env
        .services()
        .write_pending_event_as_facts(TENANT_ID, event.id)
        .await
        .expect("write again")
        .expect("a migrated event");
    assert!(matches!(second, Migration::AlreadyMigrated), "{second:?}");

    let after = written_on(&env, sub_id, day(2, 1)).await;
    assert_eq!(after.opened_components.len(), facts.opens_components.len());
    let mut conn = env.conn().await;
    let windows = SubscriptionPlanWindowRow::list_by_subscription(&mut conn, sub_id)
        .await
        .expect("plan windows");
    assert_eq!(
        windows.iter().filter(|w| w.from_date == day(2, 1)).count(),
        1,
        "one window from the change"
    );
    let row = ScheduledEventRow::get_by_id(&mut conn, event.id, &TENANT_ID)
        .await
        .expect("the event");
    assert!(matches!(row.status, DieselEventStatus::Migrated));
    drop(conn);
    assert_eq!(
        env.services()
            .resolve_pending_event(TENANT_ID, event.id)
            .await
            .expect("resolve")
            .expect_err("migrated"),
        Unresolvable::NotPending(ScheduledEventStatus::Migrated)
    );
}

#[derive(Clone, Copy, Debug)]
enum Breakage {
    TargetDraft,
    TargetInUsd,
    SeatsPriceUnlinked,
    SeatsComponentMoved,
    SubscriptionCancelled,
    CancelledOnTheChange,
    AnotherChangeLanded,
}

/// Fabricated state: what a pending Starter to Pro change finds on its target or subscription.
async fn break_it(env: &TestEnv, sub_id: SubscriptionId, breakage: Breakage) -> Unresolvable {
    use diesel_models::schema::plan_component_price::dsl as pcp;
    use diesel_models::schema::plan_version::dsl as v;
    use diesel_models::schema::price_component::dsl as pc;
    use diesel_models::schema::subscription::dsl as s;

    let mut conn = env.conn().await;
    match breakage {
        Breakage::TargetDraft => {
            diesel::update(v::plan_version.filter(v::id.eq(PLAN_VERSION_PRO_ID)))
                .set(v::is_draft_version.eq(true))
                .execute(&mut conn)
                .await
                .unwrap();
            Unresolvable::TargetVersionNotPublished(PLAN_VERSION_PRO_ID)
        }
        Breakage::TargetInUsd => {
            diesel::update(v::plan_version.filter(v::id.eq(PLAN_VERSION_PRO_ID)))
                .set(v::currency.eq("USD"))
                .execute(&mut conn)
                .await
                .unwrap();
            Unresolvable::OtherCurrency {
                subscription: "EUR".to_string(),
                target: "USD".to_string(),
            }
        }
        Breakage::SeatsPriceUnlinked => {
            diesel::delete(
                pcp::plan_component_price
                    .filter(pcp::plan_component_id.eq(COMP_PRO_SEATS_ID))
                    .filter(pcp::price_id.eq(PRICE_PRO_SEATS_ID)),
            )
            .execute(&mut conn)
            .await
            .unwrap();
            Unresolvable::PriceGone(PRICE_PRO_SEATS_ID)
        }
        Breakage::SeatsComponentMoved => {
            diesel::update(pc::price_component.filter(pc::id.eq(COMP_PRO_SEATS_ID)))
                .set(pc::plan_version_id.eq(PLAN_VERSION_STARTER_ID))
                .execute(&mut conn)
                .await
                .unwrap();
            Unresolvable::ComponentGone(COMP_PRO_SEATS_ID)
        }
        Breakage::SubscriptionCancelled => {
            diesel::update(s::subscription.filter(s::id.eq(sub_id)))
                .set(s::status.eq(DieselStatus::Cancelled))
                .execute(&mut conn)
                .await
                .unwrap();
            Unresolvable::SubscriptionNotActive(SubscriptionStatusEnum::Cancelled)
        }
        Breakage::CancelledOnTheChange => {
            env.skip_checks(
                &["I28"],
                "an ending fabricated without its cancellation event",
            );
            let at = day(2, 1).and_time(NaiveTime::MIN);
            SubscriptionEndingRow::insert_batch(
                &mut conn,
                &[SubscriptionEndingRow {
                    subscription_id: sub_id,
                    tenant_id: TENANT_ID,
                    kind: "CANCEL".to_string(),
                    at,
                    reason: None,
                    actor_type: ActorTypeEnum::System,
                    actor_uuid: None,
                    actor_alias: None,
                    created_at: at,
                }],
            )
            .await
            .unwrap();
            Unresolvable::EndsBeforeTheChange(at)
        }
        Breakage::AnotherChangeLanded => {
            env.skip_checks(
                &["I27"],
                "a plan version fabricated without its plan window",
            );
            diesel::update(s::subscription.filter(s::id.eq(sub_id)))
                .set(s::plan_version_id.eq(PLAN_VERSION_PRO_ID))
                .execute(&mut conn)
                .await
                .unwrap();
            Unresolvable::Superseded {
                expected: PLAN_VERSION_STARTER_ID,
                found: PLAN_VERSION_PRO_ID,
            }
        }
    }
}

/// A change that no longer resolves names why, and the writer writes nothing for it.
#[rstest]
#[case::target_not_published(Breakage::TargetDraft)]
#[case::target_in_another_currency(Breakage::TargetInUsd)]
#[case::price_gone(Breakage::SeatsPriceUnlinked)]
#[case::component_gone(Breakage::SeatsComponentMoved)]
#[case::subscription_ended(Breakage::SubscriptionCancelled)]
#[case::cancelled_on_the_change(Breakage::CancelledOnTheChange)]
#[case::superseded(Breakage::AnotherChangeLanded)]
#[tokio::test]
async fn a_change_that_does_not_resolve_names_why(
    #[future] test_env: TestEnv,
    #[case] breakage: Breakage,
) {
    let env = test_env.await;
    let sub_id = starter_from_jan_1(&env).await;
    let event = schedule(&env, sub_id, PLAN_VERSION_PRO_ID).await;
    let expected = break_it(&env, sub_id, breakage).await;

    let why = env
        .services()
        .resolve_pending_event(TENANT_ID, event.id)
        .await
        .expect("resolve")
        .expect_err("the change does not resolve");
    assert_eq!(why, expected, "{breakage:?}");

    let written = env
        .services()
        .write_pending_event_as_facts(TENANT_ID, event.id)
        .await
        .expect("write");
    match written {
        Err(why) => assert_eq!(why, expected, "{breakage:?}"),
        Ok(migration) => panic!("{breakage:?} wrote {migration:?}"),
    }
    let rows = written_on(&env, sub_id, day(2, 1)).await;
    assert!(rows.closed_components.is_empty() && rows.opened_components.is_empty());
    let mut conn = env.conn().await;
    let row = ScheduledEventRow::get_by_id(&mut conn, event.id, &TENANT_ID)
        .await
        .expect("the event");
    assert!(
        matches!(row.status, DieselEventStatus::Pending),
        "{breakage:?}"
    );
}

async fn status_of(env: &TestEnv, event: &ScheduledEvent) -> DieselEventStatus {
    reload_row(env, event).await.status
}

async fn reload_row(env: &TestEnv, event: &ScheduledEvent) -> ScheduledEventRow {
    ScheduledEventRow::get_by_id(&mut env.conn().await, event.id, &TENANT_ID)
        .await
        .expect("the event")
}

async fn migrate(env: &TestEnv) -> meteroid_store::services::facts::pending::PendingMigration {
    env.services()
        .migrate_pending_changes()
        .await
        .expect("the migration runs")
        .expect("this replica runs it")
}

/// An old replica claimed the change a minute ago and may be applying it: the migration leaves
/// it to the timeout. Claimed longer ago than the timeout, the claim is stale: it is migrated.
#[rstest]
#[tokio::test]
async fn the_migration_leaves_a_change_an_old_replica_is_applying(#[future] test_env: TestEnv) {
    use diesel_models::schema::scheduled_event::dsl as e;

    let env = test_env.await;
    let sub_id = starter_from_jan_1(&env).await;
    let event = schedule(&env, sub_id, PLAN_VERSION_PRO_ID).await;
    let claimed = |ago: i64| {
        diesel::update(e::scheduled_event.filter(e::id.eq(event.id))).set((
            e::status.eq(DieselEventStatus::Processing),
            e::updated_at.eq(common_clock::now().naive_utc() - chrono::Duration::minutes(ago)),
        ))
    };

    claimed(1)
        .execute(&mut env.conn().await)
        .await
        .expect("claimed a minute ago");
    let migration = migrate(&env).await;
    assert_eq!((migration.written, migration.unresolved.len()), (0, 0));
    assert!(matches!(
        status_of(&env, &event).await,
        DieselEventStatus::Processing
    ));

    claimed(10)
        .execute(&mut env.conn().await)
        .await
        .expect("claimed ten minutes ago");
    assert_eq!(migrate(&env).await.written, 1);
    assert!(matches!(
        status_of(&env, &event).await,
        DieselEventStatus::Migrated
    ));
}

/// The migration runs the tenants the billing worker runs: a paused tenant's changes, and a
/// sandbox's still being seeded, wait for it to run them.
#[rstest]
#[case::paused_tenant("UPDATE tenant SET status = 'PAUSED'")]
#[case::sandbox_seeding(
    "INSERT INTO sandbox_seed (tenant_id, status, started_at) \
     SELECT id, 'IN_PROGRESS', now() FROM tenant"
)]
#[tokio::test]
async fn the_migration_skips_the_tenants_billing_skips(
    #[future] test_env: TestEnv,
    #[case] gate: &str,
) {
    use diesel_async::SimpleAsyncConnection;

    let env = test_env.await;
    let sub_id = starter_from_jan_1(&env).await;
    let event = schedule(&env, sub_id, PLAN_VERSION_PRO_ID).await;
    env.conn()
        .await
        .batch_execute(gate)
        .await
        .expect("the tenant gated");

    let migration = migrate(&env).await;
    assert_eq!((migration.written, migration.unresolved.len()), (0, 0));
    assert!(matches!(
        status_of(&env, &event).await,
        DieselEventStatus::Pending
    ));
}

/// A change whose write fails is listed with its error; the changes after it are written.
#[rstest]
#[tokio::test]
async fn a_change_that_fails_to_write_does_not_stop_the_migration(#[future] test_env: TestEnv) {
    use diesel_models::schema::scheduled_event::dsl as e;

    let env = test_env.await;
    let broken = schedule(&env, starter_from_jan_1(&env).await, PLAN_VERSION_PRO_ID).await;
    let written = schedule(&env, starter_from_jan_1(&env).await, PLAN_VERSION_PRO_ID).await;
    diesel::update(e::scheduled_event.filter(e::id.eq(broken.id)))
        .set((
            e::event_data.eq(serde_json::json!({"ApplyPlanChange": "unreadable"})),
            e::scheduled_time.eq(day(1, 20).and_time(NaiveTime::MIN)),
        ))
        .execute(&mut env.conn().await)
        .await
        .expect("the first change unreadable");

    let migration = migrate(&env).await;
    assert_eq!(migration.written, 1);
    assert!(
        matches!(
            migration.unresolved.as_slice(),
            [(id, Unresolvable::Failed(_))] if *id == broken.id
        ),
        "{:?}",
        migration.unresolved
    );
    assert!(matches!(
        status_of(&env, &written).await,
        DieselEventStatus::Migrated
    ));
    assert!(matches!(
        reload_row(&env, &broken).await.status,
        DieselEventStatus::Pending
    ));
}

/// One replica migrates: while another holds the migration's lock, this one skips it.
#[rstest]
#[tokio::test]
async fn one_replica_runs_the_migration(#[future] test_env: TestEnv) {
    use distributed_lock::LeaderElection;
    use meteroid_store::leader_lock::{LeaderLock, PgLeaderElection};

    let env = test_env.await;
    let sub_id = starter_from_jan_1(&env).await;
    let event = schedule(&env, sub_id, PLAN_VERSION_PRO_ID).await;
    let other = PgLeaderElection::new(env.pool().clone(), LeaderLock::PendingChangesMigration)
        .try_acquire()
        .await
        .expect("election")
        .expect("the other replica holds it");

    let skipped = env
        .services()
        .migrate_pending_changes()
        .await
        .expect("the migration runs");
    assert!(skipped.is_none(), "{skipped:?}");
    assert!(matches!(
        status_of(&env, &event).await,
        DieselEventStatus::Pending
    ));

    other.release().await;
    assert_eq!(migrate(&env).await.written, 1);
}

/// The facts backfill reads a switch's plan version in its public form as the UUID it encodes:
/// one whose low 67 rotated bits are at 2^66 or more, one a hair below a carry in its rotation
/// (all ones) or in a hex digit (56 low bits set), where `/` rounded up, and new ids.
#[rstest]
#[tokio::test]
async fn the_facts_backfill_reads_a_public_plan_version_id_as_its_uuid(
    #[future] test_env: TestEnv,
) {
    use diesel::sql_types::{Text, Uuid as SqlUuid};
    use diesel_async::SimpleAsyncConnection;

    #[derive(diesel::QueryableByName)]
    struct Decoded {
        #[diesel(sql_type = diesel::sql_types::Nullable<SqlUuid>)]
        id: Option<uuid::Uuid>,
    }

    let env = test_env.await;
    let up = include_str!("../../../migrations/diesel/2026-11-15-104418_subscription_facts/up.sql");
    let from = up
        .find("CREATE FUNCTION pg_temp.plan_version_of")
        .expect("the decoding function");
    let until = from + up[from..].find("$$;").expect("its end") + "$$;".len();
    let mut conn = env.conn().await;
    conn.batch_execute(&up[from..until])
        .await
        .expect("the function is created");

    let high_rotated_bits = (1u128 << 127) + (1u128 << 66) + 12_345;
    let ids: Vec<uuid::Uuid> = [
        high_rotated_bits.rotate_right(67),
        u128::MAX,
        0x0123_4567_8900_0000_00ff_ffff_ffff_ffff,
    ]
    .into_iter()
    .map(uuid::Uuid::from_u128)
    .chain((0..20).map(|_| PlanVersionId::new().as_uuid()))
    .collect();
    let mut wrong = Vec::new();
    for id in ids {
        let public = PlanVersionId::from(id).as_base62();
        let decoded: Decoded = diesel::sql_query("SELECT pg_temp.plan_version_of($1) AS id")
            .bind::<Text, _>(&public)
            .get_result(&mut conn)
            .await
            .expect("decoded");
        if decoded.id != Some(id) {
            wrong.push(format!("{public}: {id} read as {:?}", decoded.id));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Reverting the written facts' column while a change is migrated refuses before dropping it: the
/// rollback script needs it to restore the change.
#[rstest]
#[tokio::test]
async fn the_written_facts_revert_refuses_while_a_change_is_migrated(#[future] test_env: TestEnv) {
    use diesel_async::SimpleAsyncConnection;

    let env = test_env.await;
    let sub_id = starter_from_jan_1(&env).await;
    schedule(&env, sub_id, PLAN_VERSION_PRO_ID).await;
    assert_eq!(migrate(&env).await.written, 1);

    let down = include_str!(
        "../../../migrations/diesel/2026-11-15-195248_scheduled_event_written_facts/down.sql"
    );
    let mut conn = env.conn().await;
    conn.batch_execute("BEGIN").await.expect("begin");
    let reverted = conn.batch_execute(down).await;
    conn.batch_execute("ROLLBACK").await.expect("rollback");
    assert!(reverted.is_err(), "the revert refuses");
}

/// The rollback script restores a change not applied yet, and leaves one the state applied
/// COMPLETED, a status the previous release reads.
#[rstest]
#[tokio::test]
async fn the_rollback_completes_the_changes_already_applied(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let applied = schedule(&env, starter_from_jan_1(&env).await, PLAN_VERSION_PRO_ID).await;
    assert_eq!(migrate(&env).await.written, 1);
    env.process_cycles().await;
    let pending = schedule(&env, starter_from_jan_1(&env).await, PLAN_VERSION_PRO_ID).await;
    assert_eq!(migrate(&env).await.written, 1);

    roll_back_changes_as_facts(&env).await;
    assert!(matches!(
        status_of(&env, &applied).await,
        DieselEventStatus::Completed
    ));
    assert!(matches!(
        status_of(&env, &pending).await,
        DieselEventStatus::Pending
    ));
}

/// Each plan window: its version, days and cadence.
async fn plan_windows(
    env: &TestEnv,
    sub_id: SubscriptionId,
) -> Vec<(PlanVersionId, NaiveDate, Option<NaiveDate>, String)> {
    SubscriptionPlanWindowRow::list_by_subscription(&mut env.conn().await, sub_id)
        .await
        .expect("plan windows")
        .into_iter()
        .map(|w| {
            (
                w.plan_version_id,
                w.from_date,
                w.to_date,
                format!("{:?}", w.cadence),
            )
        })
        .collect()
}

/// An annual subscription from Jan 1 whose plan window moves onto the monthly grid on Jan 1, 2026,
/// as an amendment's cadence window does (written here, no amendment opening one on a renewal day).
/// A plan change scheduled that day replaces that window; cancelled or rolled back, it gives it
/// back, and the monthly cadence with it.
#[rstest]
#[tokio::test]
async fn cancelling_a_change_restores_the_plan_window_it_replaced(#[future] test_env: TestEnv) {
    use diesel_models::subscription_facts::SubscriptionPlanWindowRowNew;

    let env = test_env.await;
    let annual = platform_fee_plan(
        &env,
        "Annual",
        DieselBillingPeriodEnum::Annual,
        Decimal::new(12_000, 2),
    )
    .await;
    let other = platform_fee_plan(
        &env,
        "Other",
        DieselBillingPeriodEnum::Annual,
        Decimal::new(24_000, 2),
    )
    .await;
    let sub_id = subscription()
        .plan_version(annual)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let next_year = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    SubscriptionPlanWindowRow::open(
        &mut env.conn().await,
        SubscriptionPlanWindowRowNew {
            id: uuid::Uuid::now_v7(),
            tenant_id: TENANT_ID,
            subscription_id: sub_id,
            plan_version_id: annual,
            from_date: next_year,
            cadence: DieselBillingPeriodEnum::Monthly,
            cadence_origin: day(1, 1),
        },
    )
    .await
    .expect("a monthly window from Jan 1, 2026");
    let amended = vec![
        (annual, day(1, 1), Some(next_year), "Annual".to_string()),
        (annual, next_year, None, "Monthly".to_string()),
    ];
    assert_eq!(plan_windows(&env, sub_id).await, amended);

    env.services()
        .schedule_plan_change(Actor::System, sub_id, TENANT_ID, other, vec![])
        .await
        .expect("a change at the period end");
    assert_eq!(
        plan_windows(&env, sub_id).await,
        vec![
            (annual, day(1, 1), Some(next_year), "Annual".to_string()),
            (other, next_year, None, "Annual".to_string()),
        ]
    );
    env.services()
        .cancel_plan_change(Actor::System, sub_id, TENANT_ID)
        .await
        .expect("the change cancelled");
    assert_eq!(plan_windows(&env, sub_id).await, amended);

    // The rollback script restores it the same way.
    env.services()
        .schedule_plan_change(Actor::System, sub_id, TENANT_ID, other, vec![])
        .await
        .expect("the change scheduled again");
    roll_back_changes_as_facts(&env).await;
    assert_eq!(plan_windows(&env, sub_id).await, amended);
}
