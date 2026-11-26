use super::*;
use crate::domain::entitlement_policy::{AtLimit, Period, Resets};
use chrono::{Duration, TimeZone};
use common_domain::ids::{
    AddOnId, BaseId, CustomerId, FeatureId, PlanId, PlanVersionId, SubscriptionId,
};
use serde_json::json;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

fn t0_now() -> DateTime<Utc> {
    t0() + Duration::days(30)
}

fn source(tier: Tier) -> EntitlementEntityId {
    match tier {
        Tier::Default => EntitlementEntityId::Feature(FeatureId::new()),
        Tier::Plan => EntitlementEntityId::Plan(PlanId::new()),
        Tier::PlanVersion => EntitlementEntityId::PlanVersion(PlanVersionId::new()),
        Tier::Subscription => EntitlementEntityId::Subscription(SubscriptionId::new()),
        Tier::AddOn => EntitlementEntityId::AddOn(AddOnId::new()),
        Tier::Customer | Tier::ResourceOverride => EntitlementEntityId::Customer(CustomerId::new()),
    }
}

fn row(tier: Tier, effect: Effect, value: GrantValue) -> FoldRow {
    FoldRow {
        lineage_id: EntitlementId::new(),
        lineage_created_at: t0(),
        lineage_anchor: t0(),
        source: source(tier),
        tier,
        effect,
        value,
        condition: None,
        condition_met: true,
        quantity: 1,
        units: None,
        policy_override: None,
        created_at: t0(),
        valid_from: None,
        valid_until: None,
        revoked_at: None,
    }
}

fn aged(mut r: FoldRow, minutes: i64) -> FoldRow {
    r.lineage_created_at = t0() + Duration::minutes(minutes);
    r
}

fn times(mut r: FoldRow, quantity: u32) -> FoldRow {
    r.quantity = quantity;
    r
}

fn cond(clauses: &[(&str, &[&str])]) -> Condition {
    Condition(
        clauses
            .iter()
            .map(|(k, vs)| (k.to_string(), vs.iter().map(|v| v.to_string()).collect()))
            .collect(),
    )
}

fn when(mut r: FoldRow, clauses: &[(&str, &[&str])], attributes: &[(&str, &str)]) -> FoldRow {
    let c = cond(clauses);
    r.condition_met = c.matches(&attrs(attributes));
    r.condition = Some(c);
    r
}

fn attrs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn with_policy(mut r: FoldRow, policy: FeaturePolicy) -> FoldRow {
    r.policy_override = Some(policy);
    r
}

fn n(v: i64) -> GrantValue {
    GrantValue::Limit(LimitValue::Amount(Decimal::from(v)))
}

fn off() -> GrantValue {
    GrantValue::Limit(LimitValue::Off)
}

fn unlimited() -> GrantValue {
    GrantValue::Limit(LimitValue::Unlimited)
}

fn text(s: &str) -> GrantValue {
    GrantValue::Text(s.to_owned())
}

fn number(v: i64) -> GrantValue {
    GrantValue::Number(Decimal::from(v))
}

fn folded(order: &ValueOrder, own: bool, rows: &[FoldRow]) -> Folded {
    fold(order, own, &FeaturePolicy::default(), rows, t0_now()).expect("rows were given")
}

fn value_of(order: &ValueOrder, rows: &[FoldRow]) -> Option<GrantValue> {
    fold(order, true, &FeaturePolicy::default(), rows, t0_now()).and_then(|f| f.value)
}

fn limit_of(rows: &[FoldRow]) -> Option<GrantValue> {
    value_of(&ValueOrder::Limit, rows)
}

fn cause_of(f: &Folded, row: &FoldRow) -> Option<Discarded> {
    f.explain
        .iter()
        .find(|c| c.lineage_id == row.lineage_id)
        .expect("every row is explained")
        .discarded
}

const ALL_TIERS: [Tier; 7] = [
    Tier::Default,
    Tier::Plan,
    Tier::PlanVersion,
    Tier::Subscription,
    Tier::AddOn,
    Tier::Customer,
    Tier::ResourceOverride,
];
const ALL_EFFECTS: [Effect; 4] = [Effect::Set, Effect::Add, Effect::RaiseTo, Effect::CapAt];

#[test]
fn every_tier_and_effect_allowed_and_refused() {
    use Effect::*;
    let ordered: [(Tier, &[Effect]); 7] = [
        (Tier::Default, &[Set]),
        (Tier::Plan, &[Set]),
        (Tier::PlanVersion, &[Set]),
        (Tier::Subscription, &[Set]),
        (Tier::AddOn, &[Add, RaiseTo]),
        (Tier::Customer, &[RaiseTo, Set, CapAt]),
        (Tier::ResourceOverride, &[RaiseTo, Set, CapAt]),
    ];
    for (order, value) in [
        (ValueOrder::Limit, n(20)),
        (ValueOrder::Unordered, text("b")),
    ] {
        for (tier, allowed) in ordered {
            for effect in ALL_EFFECTS {
                let tested = row(tier, effect, value.clone());
                let rows = [row(Tier::Default, Set, value.clone()), tested.clone()];
                let refused = cause_of(&folded(&order, true, &rows), &tested)
                    == Some(Discarded::EffectNotAllowed);
                let expected = if order.is_unordered() {
                    effect == Set
                } else {
                    allowed.contains(&effect)
                };
                assert_eq!(!refused, expected, "{order:?} {tier:?} {effect:?}");
            }
        }
    }
}

#[test]
fn default_effects() {
    let limit = ValueOrder::Limit;
    let cases = [
        (Tier::Plan, false, Effect::Set),
        (Tier::Subscription, false, Effect::Set),
        (Tier::AddOn, true, Effect::Add),
        (Tier::AddOn, false, Effect::RaiseTo),
        (Tier::Customer, false, Effect::RaiseTo),
        (Tier::ResourceOverride, false, Effect::RaiseTo),
    ];
    for (tier, multi, expected) in cases {
        assert_eq!(tier.default_effect(&limit, multi), expected, "{tier:?}");
    }
    for tier in ALL_TIERS {
        assert_eq!(
            tier.default_effect(&ValueOrder::Unordered, true),
            Effect::Set
        );
    }
}

#[test]
fn tiers_of_sources() {
    let customer = EntitlementEntityId::Customer(CustomerId::new());
    assert_eq!(Tier::of(&customer, false), Tier::Customer);
    assert_eq!(Tier::of(&customer, true), Tier::ResourceOverride);
    let quote = EntitlementEntityId::Quote(common_domain::ids::QuoteId::new());
    assert_eq!(Tier::of(&quote, false), Tier::Subscription);
}

#[test]
fn a_plan_a_pack_and_a_customer_raise() {
    let rows = [
        row(Tier::PlanVersion, Effect::Set, n(600)),
        times(row(Tier::AddOn, Effect::Add, n(300)), 2),
        row(Tier::Customer, Effect::RaiseTo, n(2000)),
    ];
    let f = folded(&ValueOrder::Limit, true, &rows);
    assert_eq!(f.value, Some(n(2000)));
    let running: Vec<_> = f.explain.iter().map(|c| c.running.clone()).collect();
    assert_eq!(running, vec![Some(n(600)), Some(n(1200)), Some(n(2000))]);
}

#[test]
fn adds_and_quantities() {
    let cases: Vec<(Vec<FoldRow>, Option<GrantValue>)> = vec![
        // Off + Add starts from 0.
        (
            vec![
                row(Tier::PlanVersion, Effect::Set, off()),
                times(row(Tier::AddOn, Effect::Add, n(1000)), 2),
            ],
            Some(n(2000)),
        ),
        (
            vec![times(row(Tier::AddOn, Effect::Add, n(5)), 3)],
            Some(n(15)),
        ),
        (
            vec![
                row(Tier::PlanVersion, Effect::Set, n(10)),
                times(row(Tier::AddOn, Effect::Add, n(5)), 0),
            ],
            Some(n(10)),
        ),
        (
            vec![
                row(Tier::PlanVersion, Effect::Set, unlimited()),
                row(Tier::AddOn, Effect::Add, n(5)),
            ],
            Some(unlimited()),
        ),
        (
            vec![
                row(Tier::PlanVersion, Effect::Set, unlimited()),
                row(Tier::AddOn, Effect::RaiseTo, n(5000)),
            ],
            Some(unlimited()),
        ),
        (
            vec![
                row(Tier::PlanVersion, Effect::Set, n(100_000)),
                row(Tier::Subscription, Effect::Set, n(1_000_000)),
                row(Tier::AddOn, Effect::RaiseTo, n(200_000)),
            ],
            Some(n(1_000_000)),
        ),
        (
            vec![
                row(Tier::PlanVersion, Effect::Set, n(1)),
                times(
                    row(
                        Tier::AddOn,
                        Effect::Add,
                        GrantValue::Limit(LimitValue::Amount(Decimal::MAX)),
                    ),
                    2,
                ),
            ],
            Some(unlimited()),
        ),
    ];
    for (rows, expected) in cases {
        assert_eq!(limit_of(&rows), expected, "{rows:#?}");
    }
}

fn per_unit(mut r: FoldRow, units: u32) -> FoldRow {
    r.units = Some(units);
    r
}

#[test]
fn a_limit_per_unit_multiplies_the_grant_by_the_units_held() {
    let plan = per_unit(row(Tier::PlanVersion, Effect::Set, n(1000)), 20);
    let rows = [
        plan.clone(),
        times(row(Tier::AddOn, Effect::Add, n(500)), 2),
        row(Tier::Customer, Effect::RaiseTo, n(15_000)),
    ];
    let f = folded(&ValueOrder::Limit, true, &rows);
    assert_eq!(f.value, Some(n(21_000)));
    let first = &f.explain[0];
    assert_eq!(
        (first.value.clone(), first.units, first.running.clone()),
        (n(1000), Some(20), Some(n(20_000)))
    );
    assert_eq!(
        limit_of(&[per_unit(row(Tier::Default, Effect::Set, n(1000)), 0)]),
        Some(n(0))
    );
    assert_eq!(
        limit_of(&[per_unit(row(Tier::Plan, Effect::Set, unlimited()), 3)]),
        Some(unlimited())
    );
    // Units scale before Set rows of one tier compare.
    let better = per_unit(aged(row(Tier::Plan, Effect::Set, n(10)), 1), 5);
    let worse = per_unit(row(Tier::Plan, Effect::Set, n(9)), 5);
    assert_eq!(limit_of(&[worse, better]), Some(n(50)));
}

#[test]
fn an_add_that_changes_nothing_is_explained_as_no_effect() {
    let pack = row(Tier::AddOn, Effect::Add, n(5));
    let zero = times(row(Tier::AddOn, Effect::Add, n(5)), 0);
    let rows = [
        row(Tier::PlanVersion, Effect::Set, unlimited()),
        pack.clone(),
        zero.clone(),
    ];
    let f = folded(&ValueOrder::Limit, true, &rows);
    assert_eq!(cause_of(&f, &pack), Some(Discarded::NoEffect));
    assert_eq!(cause_of(&f, &zero), Some(Discarded::NoEffect));
}

#[test]
fn a_customer_set_discards_paid_add_ons() {
    let pack = row(Tier::AddOn, Effect::Add, n(30));
    let rows = [
        row(Tier::PlanVersion, Effect::Set, n(50)),
        pack.clone(),
        row(Tier::Customer, Effect::Set, n(10)),
    ];
    let f = folded(&ValueOrder::Limit, true, &rows);
    assert_eq!(f.value, Some(n(10)));
    assert_eq!(cause_of(&f, &pack), Some(Discarded::ReplacedByHigherTier));
}

#[test]
fn customer_overrides_across_plan_changes() {
    // Before (plan 50), after an upgrade to 200, after a downgrade to 20, with an override of 100 (§3.2).
    let cases = [
        (Effect::RaiseTo, [100, 200, 100]),
        (Effect::CapAt, [50, 100, 20]),
        (Effect::Set, [100, 100, 100]),
    ];
    for own in [true, false] {
        for (effect, expected) in cases {
            let got = [50, 200, 20].map(|plan| {
                fold(
                    &ValueOrder::Limit,
                    own,
                    &FeaturePolicy::default(),
                    &[
                        row(Tier::PlanVersion, Effect::Set, n(plan)),
                        row(Tier::Customer, effect, n(100)),
                    ],
                    t0_now(),
                )
                .and_then(|f| f.value)
            });
            assert_eq!(got, expected.map(|v| Some(n(v))), "{effect:?} own={own}");
        }
    }
}

#[test]
fn within_a_tier_raise_comes_before_cap() {
    let rows = [
        row(Tier::PlanVersion, Effect::Set, n(50)),
        row(Tier::Customer, Effect::CapAt, n(80)),
        row(Tier::Customer, Effect::RaiseTo, n(100)),
    ];
    assert_eq!(limit_of(&rows), Some(n(80)));
}

#[test]
fn lone_rows_on_the_own_level_and_on_another() {
    let cases = [
        (
            true,
            Tier::Customer,
            Effect::CapAt,
            None,
            Some(Discarded::NoEffect),
        ),
        (true, Tier::Customer, Effect::RaiseTo, Some(n(100)), None),
        (true, Tier::AddOn, Effect::Add, Some(n(100)), None),
        (false, Tier::Customer, Effect::CapAt, Some(n(100)), None),
        (
            false,
            Tier::Customer,
            Effect::RaiseTo,
            None,
            Some(Discarded::NoBaseOnLevel),
        ),
        (
            false,
            Tier::AddOn,
            Effect::RaiseTo,
            None,
            Some(Discarded::NoBaseOnLevel),
        ),
        (
            false,
            Tier::AddOn,
            Effect::Add,
            None,
            Some(Discarded::NoBaseOnLevel),
        ),
    ];
    for (own, tier, effect, value, cause) in cases {
        let lone = row(tier, effect, n(100));
        let f = folded(&ValueOrder::Limit, own, std::slice::from_ref(&lone));
        assert_eq!(f.value, value, "own={own} {effect:?}");
        assert_eq!(cause_of(&f, &lone), cause, "own={own} {effect:?}");
    }
}

#[test]
fn effects_on_another_level_follow_its_base() {
    let other = |rows: &[FoldRow]| folded(&ValueOrder::Limit, false, rows).value;
    // A pack adds 10 across all sites.
    assert_eq!(
        other(&[
            row(Tier::PlanVersion, Effect::Set, n(50)),
            row(Tier::AddOn, Effect::Add, n(10)),
        ]),
        Some(n(60))
    );
    // A customer Raise to 80 follows an upgrade to 100.
    for (plan, expected) in [(50, 80), (100, 100)] {
        assert_eq!(
            other(&[
                row(Tier::PlanVersion, Effect::Set, n(plan)),
                row(Tier::Customer, Effect::RaiseTo, n(80)),
            ]),
            Some(n(expected))
        );
    }
    assert!(
        fold(
            &ValueOrder::Limit,
            false,
            &FeaturePolicy::default(),
            &[],
            t0_now()
        )
        .is_none()
    );
}

#[test]
fn caps_switch_off() {
    let rows = [
        row(Tier::PlanVersion, Effect::Set, n(50)),
        row(Tier::Customer, Effect::CapAt, off()),
    ];
    assert_eq!(limit_of(&rows), Some(off()));
    let customer_off = [row(
        Tier::Customer,
        Effect::CapAt,
        GrantValue::Boolean(false),
    )];
    assert_eq!(
        folded(&ValueOrder::Boolean, false, &customer_off).value,
        Some(GrantValue::Boolean(false))
    );
}

#[test]
fn conditions_pick_among_sets_by_specificity() {
    let env = "site.environment";
    let staging = [(env, "staging"), ("site.region", "eu")];
    let cases: Vec<(Vec<FoldRow>, GrantValue)> = vec![
        (
            vec![
                row(Tier::Default, Effect::Set, n(10)),
                when(
                    row(Tier::Default, Effect::Set, n(1)),
                    &[(env, &["staging", "qa"])],
                    &staging,
                ),
            ],
            n(1),
        ),
        (
            vec![
                when(
                    row(Tier::Default, Effect::Set, n(1)),
                    &[(env, &["staging", "qa"])],
                    &staging,
                ),
                when(
                    row(Tier::Default, Effect::Set, n(2)),
                    &[(env, &["staging"])],
                    &staging,
                ),
            ],
            n(2),
        ),
        (
            vec![
                when(
                    row(Tier::Default, Effect::Set, n(2)),
                    &[(env, &["staging"])],
                    &staging,
                ),
                when(
                    row(Tier::Default, Effect::Set, n(3)),
                    &[(env, &["staging", "qa"]), ("site.region", &["eu"])],
                    &staging,
                ),
            ],
            n(3),
        ),
        // Specificity never discards an Add.
        (
            vec![
                when(
                    row(Tier::AddOn, Effect::Add, n(10)),
                    &[(env, &["staging"])],
                    &staging,
                ),
                row(Tier::AddOn, Effect::Add, n(5)),
            ],
            n(15),
        ),
        // A missing attribute doesn't match: the plain row answers.
        (
            vec![
                row(Tier::Default, Effect::Set, n(10)),
                when(
                    row(Tier::Default, Effect::Set, n(1)),
                    &[("site.tier", &["gold"])],
                    &staging,
                ),
            ],
            n(10),
        ),
    ];
    for (rows, expected) in cases {
        assert_eq!(limit_of(&rows), Some(expected), "{rows:#?}");
    }
}

#[test]
fn an_unmatched_condition_and_dates_are_explained() {
    let unmatched = when(
        row(Tier::Default, Effect::Set, n(1)),
        &[("site.environment", &["staging"])],
        &[("site.environment", "production")],
    );
    let mut expired = row(Tier::Customer, Effect::RaiseTo, n(500));
    expired.valid_until = Some(t0_now());
    let rows = [
        row(Tier::Default, Effect::Set, n(10)),
        unmatched.clone(),
        expired.clone(),
    ];
    let f = folded(&ValueOrder::Limit, true, &rows);
    assert_eq!(f.value, Some(n(10)));
    assert_eq!(cause_of(&f, &unmatched), Some(Discarded::ConditionNotMet));
    assert_eq!(cause_of(&f, &expired), Some(Discarded::NotInEffect));
}

#[test]
fn conditions_need_every_attribute() {
    let c = cond(&[
        ("tenant.environment", &["sandbox"]),
        ("tenant.region", &["eu"]),
    ]);
    assert!(c.matches(&attrs(&[
        ("tenant.environment", "sandbox"),
        ("tenant.region", "eu")
    ])));
    assert!(!c.matches(&attrs(&[("tenant.environment", "sandbox")])));
    assert!(!c.matches(&attrs(&[
        ("tenant.environment", "production"),
        ("tenant.region", "eu")
    ])));
    assert!(Condition::default().matches(&attrs(&[])));
}

#[test]
fn condition_hashes_are_canonical() {
    let a = cond(&[
        ("site.environment", &["staging", "qa"]),
        ("site.region", &["eu"]),
    ]);
    let b = cond(&[
        ("site.region", &["eu"]),
        ("site.environment", &["qa", "staging", "qa"]),
    ]);
    assert_eq!(a.hash(), b.hash());
    assert_eq!(a.hash().len(), 64);
    assert_ne!(a.hash(), cond(&[("site.environment", &["staging"])]).hash());
    assert_eq!(Condition::hash_of(None), "");
    assert_eq!(Condition::default().hash(), "");
    assert_eq!(Condition::hash_of(Some(&a)), a.hash());
}

#[test]
fn same_tier_sets_take_the_best_then_the_oldest() {
    let rows = [
        aged(row(Tier::PlanVersion, Effect::Set, n(10)), 0),
        aged(row(Tier::PlanVersion, Effect::Set, n(30)), 5),
    ];
    assert_eq!(limit_of(&rows), Some(n(30)));

    let older = aged(row(Tier::Customer, Effect::Set, text("a")), 0);
    let newer = aged(row(Tier::Customer, Effect::Set, text("b")), 5);
    let f = folded(&ValueOrder::Unordered, true, &[newer.clone(), older]);
    assert_eq!(f.value, Some(text("a")));
    assert_eq!(cause_of(&f, &newer), Some(Discarded::LostWithinTier));
}

#[test]
fn ordered_values() {
    let ranked = ValueOrder::Ranked(vec!["basic".into(), "standard".into(), "advanced".into()]);
    let lower = ValueOrder::Number {
        higher_is_better: false,
    };
    let higher = ValueOrder::Number {
        higher_is_better: true,
    };
    let (on, no) = (GrantValue::Boolean(true), GrantValue::Boolean(false));
    let cases: Vec<(ValueOrder, Vec<FoldRow>, GrantValue)> = vec![
        (
            ranked.clone(),
            vec![
                row(Tier::PlanVersion, Effect::Set, text("standard")),
                row(Tier::AddOn, Effect::RaiseTo, text("advanced")),
                row(Tier::Customer, Effect::RaiseTo, text("basic")),
            ],
            text("advanced"),
        ),
        // Add on a value that doesn't add up raises at most.
        (
            ranked,
            vec![
                row(Tier::PlanVersion, Effect::Set, text("standard")),
                row(Tier::AddOn, Effect::Add, text("basic")),
            ],
            text("standard"),
        ),
        (
            lower.clone(),
            vec![
                row(Tier::PlanVersion, Effect::Set, number(10)),
                row(Tier::AddOn, Effect::RaiseTo, number(5)),
                row(Tier::Customer, Effect::RaiseTo, number(8)),
            ],
            number(5),
        ),
        (
            lower,
            vec![
                row(Tier::PlanVersion, Effect::Set, number(100)),
                row(Tier::AddOn, Effect::Add, number(50)),
            ],
            number(50),
        ),
        (
            higher,
            vec![
                row(Tier::PlanVersion, Effect::Set, number(600)),
                times(row(Tier::AddOn, Effect::Add, number(300)), 2),
            ],
            number(1200),
        ),
        (
            ValueOrder::Boolean,
            vec![
                row(Tier::PlanVersion, Effect::Set, no.clone()),
                row(Tier::AddOn, Effect::Add, on.clone()),
            ],
            on.clone(),
        ),
        (
            ValueOrder::Boolean,
            vec![
                row(Tier::PlanVersion, Effect::Set, no.clone()),
                row(Tier::AddOn, Effect::Add, no.clone()),
            ],
            no.clone(),
        ),
        (
            ValueOrder::Boolean,
            vec![
                row(Tier::PlanVersion, Effect::Set, on),
                row(Tier::Customer, Effect::CapAt, no.clone()),
            ],
            no,
        ),
    ];
    for (order, rows, expected) in cases {
        assert_eq!(value_of(&order, &rows), Some(expected), "{order:?}");
    }
}

#[test]
fn the_policy_is_kept_across_tiers() {
    let feature = FeaturePolicy::default();
    let enterprise = FeaturePolicy {
        at_limit: AtLimit::AllowAndBill,
        ..FeaturePolicy::default()
    };
    let custom = FeaturePolicy {
        resets: Resets::Calendar,
        period: Some(Period {
            unit: crate::domain::entitlements::PeriodUnit::Month,
            interval: 1,
        }),
        ..FeaturePolicy::default()
    };
    let plan = with_policy(
        row(Tier::PlanVersion, Effect::Set, n(1000)),
        enterprise.clone(),
    );
    let set_custom = with_policy(row(Tier::Customer, Effect::Set, n(3000)), custom.clone());
    let losing = with_policy(
        aged(row(Tier::Customer, Effect::Set, n(10)), 5),
        custom.clone(),
    );
    let pack = with_policy(row(Tier::AddOn, Effect::Add, n(5)), custom.clone());
    let cases: Vec<(Vec<FoldRow>, &FeaturePolicy, Option<EntitlementId>)> = vec![
        (
            vec![row(Tier::PlanVersion, Effect::Set, n(1000))],
            &feature,
            None,
        ),
        (
            vec![plan.clone(), row(Tier::Customer, Effect::RaiseTo, n(5000))],
            &enterprise,
            Some(plan.lineage_id),
        ),
        (
            vec![plan.clone(), row(Tier::Customer, Effect::CapAt, n(10))],
            &enterprise,
            Some(plan.lineage_id),
        ),
        (
            vec![plan.clone(), row(Tier::Customer, Effect::Set, n(2000))],
            &enterprise,
            Some(plan.lineage_id),
        ),
        (
            vec![plan.clone(), set_custom.clone()],
            &custom,
            Some(set_custom.lineage_id),
        ),
        (
            vec![
                plan.clone(),
                row(Tier::Customer, Effect::Set, n(3000)),
                losing,
            ],
            &enterprise,
            Some(plan.lineage_id),
        ),
        (vec![plan.clone(), pack], &enterprise, Some(plan.lineage_id)),
    ];
    for (rows, policy, from) in cases {
        let f = folded(&ValueOrder::Limit, true, &rows);
        assert_eq!((&f.policy, f.policy_from), (policy, from), "{rows:#?}");
    }
}

#[test]
fn liveness_follows_dates_and_revocation() {
    let mut r = row(Tier::Customer, Effect::RaiseTo, n(1));
    let day = |d| t0() + Duration::days(d);
    r.created_at = day(10);
    r.valid_until = Some(day(20));
    assert!(!r.is_live(day(5)));
    assert!(r.is_live(day(15)));
    assert!(!r.is_live(day(20)));
    r.revoked_at = Some(day(12));
    assert!(r.is_live(day(11)));
    assert!(!r.is_live(day(12)));
    r.valid_from = Some(day(11));
    assert!(!r.is_live(day(10) + Duration::hours(12)));
}

#[test]
fn levels_normalise_to_one_spelling() {
    let site = common_domain::ids::ResourceTypeId::new();
    let tenant = common_domain::ids::ResourceTypeId::new();
    let cases = [
        (
            RowLevel::Type(site),
            Some(site),
            RowLevel::Feature,
            EvalLevel::Type(site),
        ),
        (
            RowLevel::Customer,
            None,
            RowLevel::Feature,
            EvalLevel::Customer,
        ),
        (
            RowLevel::Customer,
            Some(site),
            RowLevel::Customer,
            EvalLevel::Customer,
        ),
        (
            RowLevel::Type(tenant),
            Some(site),
            RowLevel::Type(tenant),
            EvalLevel::Type(tenant),
        ),
        (
            RowLevel::Feature,
            Some(site),
            RowLevel::Feature,
            EvalLevel::Type(site),
        ),
        (
            RowLevel::Feature,
            None,
            RowLevel::Feature,
            EvalLevel::Customer,
        ),
    ];
    for (level, feature_level, normalised, eval) in cases {
        assert_eq!(level.normalised(feature_level), normalised, "{level:?}");
        assert_eq!(level.eval(feature_level), eval, "{level:?}");
    }
}

#[test]
fn values_read_main_s_stored_shapes_and_round_trip() {
    let cases = [
        (
            json!({"type": "metered", "limit": "5", "reset_period": {"type": "never"},
                   "overage_behavior": {"type": "allow"}, "enabled": true}),
            n(5),
        ),
        (
            json!({"type": "metered", "limit": "5", "enabled": false}),
            off(),
        ),
        (json!({"type": "metered"}), unlimited()),
        (
            json!({"type": "boolean", "enabled": true}),
            GrantValue::Boolean(true),
        ),
        (
            json!({"type": "config", "value": {"kind": "boolean", "value": false}}),
            GrantValue::Boolean(false),
        ),
        (
            json!({"type": "config", "value": {"kind": "number", "value": "7"}}),
            number(7),
        ),
        (
            json!({"type": "config", "value": {"kind": "text", "value": "gold"}}),
            text("gold"),
        ),
        (
            json!({"type": "config", "value": {"kind": "json", "value": {"a": 1}}}),
            GrantValue::Json(json!({"a": 1})),
        ),
    ];
    for (stored, expected) in cases {
        let read: GrantValue = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(read, expected, "{stored}");
        let written = serde_json::to_value(&read).unwrap();
        assert_eq!(
            serde_json::from_value::<GrantValue>(written).unwrap(),
            expected
        );
    }
}

/// A small deterministic generator: the crate has no proptest.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[(self.next() % items.len() as u64) as usize]
    }

    fn value(&mut self, order: &ValueOrder) -> GrantValue {
        let k = self.next();
        match order {
            ValueOrder::Limit => match k % 10 {
                0 => unlimited(),
                1 => off(),
                k => n((k * 100 + self.next() % 100) as i64),
            },
            ValueOrder::Boolean => GrantValue::Boolean(k.is_multiple_of(2)),
            ValueOrder::Number { .. } => number((k % 1000) as i64),
            ValueOrder::Ranked(options) => text(&options[(k % options.len() as u64) as usize]),
            ValueOrder::Unordered => text("x"),
        }
    }

    fn rows(&mut self, order: &ValueOrder) -> Vec<FoldRow> {
        (0..1 + self.next() % 7)
            .map(|i| {
                let (tier, effect) = (self.pick(&ALL_TIERS), self.pick(&ALL_EFFECTS));
                let value = self.value(order);
                times(
                    aged(row(tier, effect, value), i as i64),
                    (self.next() % 4) as u32,
                )
            })
            .collect()
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, (self.next() % (i as u64 + 1)) as usize);
        }
    }
}

fn ordered_orders() -> Vec<ValueOrder> {
    vec![
        ValueOrder::Limit,
        ValueOrder::Boolean,
        ValueOrder::Number {
            higher_is_better: true,
        },
        ValueOrder::Number {
            higher_is_better: false,
        },
        ValueOrder::Ranked(vec!["basic".into(), "pro".into(), "enterprise".into()]),
    ]
}

#[test]
fn the_result_never_depends_on_row_order() {
    let mut rng = Lcg(7);
    for order in ordered_orders() {
        for own in [true, false] {
            for _ in 0..1_000 {
                let rows = rng.rows(&order);
                let expected = folded(&order, own, &rows).value;
                let mut shuffled = rows.clone();
                rng.shuffle(&mut shuffled);
                assert_eq!(
                    folded(&order, own, &shuffled).value,
                    expected,
                    "{order:?} {rows:#?}"
                );
            }
        }
    }
}

#[test]
fn buying_an_add_on_never_lowers_the_value() {
    let mut rng = Lcg(11);
    for order in ordered_orders() {
        for _ in 0..2_000 {
            let mut rows = rng.rows(&order);
            // A customer or resource Set to forces whatever is below it, add-ons included.
            rows.retain(|r| !(r.tier >= Tier::Customer && r.effect == Effect::Set));
            let before = value_of(&order, &rows);
            let effect = rng.pick(&[Effect::Add, Effect::RaiseTo]);
            let value = rng.value(&order);
            rows.push(times(
                row(Tier::AddOn, effect, value),
                (rng.next() % 3) as u32,
            ));
            if let (Some(before), Some(after)) = (&before, &value_of(&order, &rows)) {
                assert_ne!(
                    order.compare(after, before),
                    Some(Ordering::Less),
                    "{order:?}: {before:?} → {after:?}; {rows:#?}"
                );
            }
        }
    }
}

#[test]
fn a_v1_metered_value_reads_with_its_v1_fields_ignored() {
    let v1 = json!({"type": "metered", "limit": "10", "enabled": true, "thresholds": [80],
        "reset_period": {"type": "billing_cycle"}, "group_by": "site",
        "overage_behavior": {"type": "block", "grace_period_pct": 10}});
    let value: GrantValue = serde_json::from_value(v1).unwrap();
    assert_eq!(
        value,
        GrantValue::Limit(LimitValue::Amount(Decimal::from(10)))
    );
}
