use std::collections::BTreeMap;

use chrono::{DateTime, Days, Months, NaiveDate, Utc};
use common_domain::ids::{BillableMetricId, CreditUnitId, ProductId};
use proptest::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use uuid::Uuid;

use super::*;
use crate::facts::{CouponFact, FactKind, MinimumFact, PlanWindow, SeatStep, Start, Trial};
use crate::issued::{IssuedKind, IssuedLine};
use crate::reconcile::{Adjustment, TakeBack, reconcile};
use crate::timeline::Grid;

#[derive(Clone, Debug, Default, PartialEq)]
struct TestFee {
    /// A whole period's price, per seat.
    price: i64,
    metric: Option<BillableMetricId>,
    /// Per unit of usage.
    unit: i64,
    /// Priced in credits of this unit.
    credit_unit: Option<CreditUnitId>,
    /// A pack's credit for a whole period, per seat.
    grant: Option<i64>,
    once_product: Option<ProductId>,
    /// Its price is out of range.
    overflows: bool,
    /// Billed whole, never prorated (a capacity, a pack with its own duration).
    whole: bool,
    /// Units of usage included before `unit` applies, for a whole period (a capacity's).
    included: i64,
    /// Graduated: past this many billable units, each costs the second figure.
    tier: Option<(i64, i64)>,
    /// Priced once per period, not part by part.
    per_period: bool,
}

impl TestFee {
    fn denomination(&self) -> Denomination {
        self.credit_unit
            .map_or(Denomination::Money, Denomination::CreditUnit)
    }
}

struct TestPricer;

impl Pricer<TestFee> for TestPricer {
    fn metrics(&self, fee: &TestFee) -> Vec<BillableMetricId> {
        fee.metric.into_iter().collect()
    }

    fn per_part(&self, fee: &TestFee) -> bool {
        !fee.per_period
    }

    fn shares_allowance(&self, earlier: &TestFee, later: &TestFee) -> bool {
        earlier.included == later.included
    }

    fn rates(&self, segment: &Segment<'_, TestFee>) -> Result<Vec<Rate>, PricingError> {
        segment
            .fees
            .iter()
            .filter(|fee| fee.fee.metric.is_none())
            .map(|fee| {
                if fee.fee.overflows {
                    return Err(PricingError::OutOfRange);
                }
                let seats = i64::try_from(fee.seats.unwrap_or(1)).unwrap();
                Ok(Rate {
                    fee: fee.lineage.clone(),
                    kind: if fee.fee.grant.is_some() {
                        ChargeKind::Credit
                    } else {
                        ChargeKind::Fee
                    },
                    quantity: Some(Decimal::from(seats)),
                    amount: fee.fee.price * seats,
                    denomination: fee.fee.denomination(),
                    prorated: !fee.fee.whole,
                    credit_grant: fee.fee.grant.map(|grant| CreditGrant {
                        amount: grant * seats,
                        denomination: Denomination::Money,
                        prorated: !fee.fee.whole,
                    }),
                    once_per_product: fee.fee.once_product,
                    shown: Default::default(),
                })
            })
            .collect()
    }

    fn usage(
        &self,
        segment: &Segment<'_, TestFee>,
        window: &UsageWindow,
        reads: &UsageReads,
    ) -> Result<Vec<Rate>, PricingError> {
        segment
            .fees
            .iter()
            .filter_map(|fee| {
                let metric = fee.fee.metric?;
                if fee.fee.overflows {
                    return Some(Err(PricingError::OutOfRange));
                }
                let usage = reads.get(&UsageRead {
                    metric,
                    window: *window,
                })?;
                let total: Decimal = usage.iter().map(|(_, quantity)| *quantity).sum();
                let included = fee
                    .allowance
                    .prorate_decimal(Decimal::from(fee.fee.included));
                let billable = (total - included).max(Decimal::ZERO);
                let amount = match fee.fee.tier {
                    Some((upto, after)) => {
                        let upto = Decimal::from(upto);
                        billable.min(upto) * Decimal::from(fee.fee.unit)
                            + (billable - upto).max(Decimal::ZERO) * Decimal::from(after)
                    }
                    None => billable * Decimal::from(fee.fee.unit),
                };
                Some(Ok(Rate {
                    fee: fee.lineage.clone(),
                    kind: ChargeKind::Usage {
                        metric,
                        dimensions: BTreeMap::new(),
                    },
                    quantity: Some(total),
                    amount: amount.round().to_i64().unwrap(),
                    denomination: fee.fee.denomination(),
                    prorated: false,
                    credit_grant: None,
                    once_per_product: None,
                    shown: Default::default(),
                }))
            })
            .collect()
    }
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn at(y: i32, m: u32, day: u32, h: u32) -> DateTime<Utc> {
    d(y, m, day).and_hms_opt(h, 0, 0).unwrap().and_utc()
}

fn id<T: From<Uuid>>(n: u128) -> T {
    Uuid::from_u128(n).into()
}

const SUBSCRIPTION: u128 = 1;
const METRIC: u128 = 900;

fn lineage(n: u128) -> Fee {
    Fee::Component(id(n))
}

fn plan(version: u128, from: NaiveDate, cadence: u32, origin: NaiveDate) -> PlanWindow {
    PlanWindow {
        plan_version: id(version),
        from,
        to: None,
        cadence,
        cadence_origin: origin,
    }
}

fn subscription(start: NaiveDate, anchor: u32, cadence: u32) -> Facts<TestFee> {
    Facts {
        subscription: id(SUBSCRIPTION),
        billing_day_anchor: anchor,
        start: Start {
            date: start,
            settled: Some(midnight(start)),
            checkout: false,
        },
        trial: None,
        plan_windows: vec![plan(100, start, cadence, start)],
        fees: Vec::new(),
        seats: Vec::new(),
        coupons: Vec::new(),
        minimums: Vec::new(),
        ending: None,
        end_date: None,
        reconcile_from: None,
        arrears_from: None,
        ending_recorded: None,
        ending_day_invoiced: false,
    }
}

fn fixed(row: u128, of: u128, from: NaiveDate, cadence: u32, price: i64) -> FeeFact<TestFee> {
    FeeFact {
        id: id(row),
        lineage: lineage(of),
        from,
        to: None,
        billing: Billing::Advance,
        cadence,
        plan_component: true,
        fee: TestFee {
            price,
            ..TestFee::default()
        },
    }
}

fn metered(row: u128, of: u128, from: NaiveDate, cadence: u32, unit: i64) -> FeeFact<TestFee> {
    FeeFact {
        id: id(row),
        lineage: lineage(of),
        from,
        to: None,
        billing: Billing::Arrears,
        cadence,
        plan_component: true,
        fee: TestFee {
            metric: Some(id(METRIC)),
            unit,
            ..TestFee::default()
        },
    }
}

fn read(from: DateTime<Utc>, until: DateTime<Utc>) -> UsageRead {
    UsageRead {
        metric: id(METRIC),
        window: UsageWindow { from, until },
    }
}

fn reads(entries: &[(UsageRead, i64)]) -> UsageReads {
    entries
        .iter()
        .map(|(read, quantity)| (*read, vec![(BTreeMap::new(), Decimal::from(*quantity))]))
        .collect()
}

fn run(facts: &Facts<TestFee>, now: DateTime<Utc>) -> Evaluation {
    evaluate(facts, now, None, &TestPricer).unwrap()
}

fn of(evaluation: &Evaluation, fee: u128) -> Vec<&Charge> {
    evaluation
        .charges
        .iter()
        .filter(|charge| charge.key.fee == lineage(fee))
        .collect()
}

fn periods(charges: &[&Charge]) -> Vec<(NaiveDate, NaiveDate)> {
    charges
        .iter()
        .map(|charge| (charge.key.from, charge.key.until))
        .collect()
}

fn recurring(on: NaiveDate) -> DocumentSlot {
    DocumentSlot::Recurring { period_start: on }
}

/// Rules: R-084
#[test]
fn a_month_end_anchor_renews_on_each_months_last_day() {
    let mut facts = subscription(d(2025, 1, 31), 31, 1);
    facts.fees.push(fixed(10, 1, d(2025, 1, 31), 1, 3_100));
    let evaluation = run(&facts, at(2025, 4, 15, 0));
    let charges = of(&evaluation, 1);
    assert_eq!(
        periods(&charges),
        vec![
            (d(2025, 1, 31), d(2025, 2, 28)),
            (d(2025, 2, 28), d(2025, 3, 31)),
            (d(2025, 3, 31), d(2025, 4, 30)),
        ]
    );
    assert!(charges.iter().all(|charge| charge.amount == 3_100));
    assert_eq!(charges[1].document, recurring(d(2025, 2, 28)));
    assert_eq!(evaluation.next_wake, Some(at(2025, 4, 30, 0)));
    assert_eq!(evaluation.state.status, Status::Active);
    assert_eq!(
        evaluation.state.current_period,
        Some((d(2025, 3, 31), Some(d(2025, 4, 30))))
    );
}

/// Rules: R-084
#[test]
fn a_yearly_subscription_from_a_leap_day_renews_on_the_last_day_of_february() {
    let mut facts = subscription(d(2024, 2, 29), 29, 12);
    facts.fees.push(fixed(10, 1, d(2024, 2, 29), 12, 12_000));
    let evaluation = run(&facts, at(2026, 3, 1, 0));
    let charges = of(&evaluation, 1);
    assert_eq!(
        periods(&charges),
        vec![
            (d(2024, 2, 29), d(2025, 2, 28)),
            (d(2025, 2, 28), d(2026, 2, 28)),
            (d(2026, 2, 28), d(2027, 2, 28)),
        ]
    );
    assert!(charges.iter().all(|charge| charge.amount == 12_000));
}

/// A free trial to January 15 on a grid anchored on the 1st: billing starts there, its first
/// period up to February 1 priced as 17 of January's 31 days.
/// Rules: R-084, R-011
#[test]
fn a_free_trial_bills_nothing_then_bills_from_its_end() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.trial = Some(Trial {
        from: d(2024, 1, 1),
        until: d(2024, 1, 15),
        free: true,
    });
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 3_100));

    let trialing = run(&facts, at(2024, 1, 10, 0));
    assert!(trialing.charges.is_empty());
    assert_eq!(trialing.state.status, Status::Trialing);
    assert_eq!(
        trialing.state.current_period,
        Some((d(2024, 1, 1), Some(d(2024, 1, 15))))
    );
    assert_eq!(trialing.next_wake, Some(at(2024, 1, 15, 0)));

    let paid = run(&facts, at(2024, 1, 20, 0));
    assert_eq!(paid.state.status, Status::Active);
    assert_eq!(
        paid.state.first_period,
        Some((d(2024, 1, 15), d(2024, 2, 1)))
    );
    let charges = of(&paid, 1);
    assert_eq!(charges.len(), 1);
    assert_eq!(charges[0].amount, 1_700);
    assert_eq!(charges[0].document, recurring(d(2024, 1, 15)));
    assert!(charges[0].trace.contains(&FactRef {
        kind: FactKind::Trial,
        id: Uuid::nil()
    }));
}

/// Unsettled after its free trial, a subscription bills its first period on the checkout's
/// invoice, and no renewal after it however long the checkout waits.
#[test]
fn a_free_trial_ended_before_its_checkout_settled_expires() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.start.settled = None;
    facts.start.checkout = true;
    facts.trial = Some(Trial {
        from: d(2024, 1, 1),
        until: d(2024, 1, 15),
        free: true,
    });
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 3_100));
    assert_eq!(
        run(&facts, at(2024, 1, 10, 0)).state.status,
        Status::Trialing
    );
    let expired = run(&facts, at(2024, 1, 20, 0));
    assert_eq!(expired.state.status, Status::TrialExpired);
    assert!(expired.state.pending_checkout);
    assert_eq!(of(&expired, 1)[0].amount, 1_700);

    let months_later = run(&facts, at(2024, 4, 10, 0));
    assert_eq!(months_later.state.status, Status::TrialExpired);
    let charges = of(&months_later, 1);
    assert_eq!(periods(&charges), vec![(d(2024, 1, 15), d(2024, 2, 1))]);
    assert_eq!(charges[0].document, recurring(d(2024, 1, 15)));

    facts.start.settled = Some(at(2024, 4, 10, 0));
    let paid = run(&facts, at(2024, 4, 10, 1));
    assert_eq!(paid.state.status, Status::Active);
    assert_eq!(of(&paid, 1).len(), 4);
}

/// A paid trial bills like any period; only the status tells it apart.
#[test]
fn a_paid_trial_bills_from_the_start() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.trial = Some(Trial {
        from: d(2024, 1, 1),
        until: d(2024, 1, 15),
        free: false,
    });
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 3_100));
    let evaluation = run(&facts, at(2024, 1, 10, 0));
    assert_eq!(evaluation.state.status, Status::Trialing);
    assert_eq!(of(&evaluation, 1)[0].amount, 3_100);
    assert_eq!(of(&evaluation, 1)[0].document, recurring(d(2024, 1, 1)));
}

/// The checkout's invoice is computed before the payment settling the start: the first period
/// is billed from the start's date, flagged, and nothing after it until the start settles.
/// Rules: R-084
#[test]
fn a_checkout_start_not_settled_bills_its_first_period_alone() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.start.settled = None;
    facts.start.checkout = true;
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 3_100));
    facts.fees.push(metered(20, 2, d(2024, 1, 1), 1, 1));
    let pending = run(&facts, at(2024, 3, 10, 0));
    assert_eq!(pending.state.status, Status::Pending);
    assert!(pending.state.pending_checkout);
    assert_eq!(pending.state.current_period, Some((d(2024, 1, 1), None)));
    assert_eq!(
        periods(&of(&pending, 1)),
        vec![(d(2024, 1, 1), d(2024, 2, 1))]
    );
    assert!(
        pending
            .charges
            .iter()
            .all(|charge| charge.document == recurring(d(2024, 1, 1)))
    );
    assert!(pending.usage_needed.is_empty());

    // Settled months later, it bills from its start's date on.
    facts.start.settled = Some(at(2024, 3, 10, 0));
    let settled = run(&facts, at(2024, 3, 10, 1));
    assert_eq!(settled.state.status, Status::Active);
    assert!(!settled.state.pending_checkout);
    assert_eq!(
        periods(&of(&settled, 1))[..3],
        [
            (d(2024, 1, 1), d(2024, 2, 1)),
            (d(2024, 2, 1), d(2024, 3, 1)),
            (d(2024, 3, 1), d(2024, 4, 1)),
        ]
    );

    let mut later = subscription(d(2024, 3, 1), 1, 1);
    later.fees.push(fixed(10, 1, d(2024, 3, 1), 1, 3_100));
    let before = run(&later, at(2024, 2, 1, 0));
    assert_eq!(before.state.status, Status::Pending);
    assert!(!before.state.pending_checkout);
    assert_eq!(before.next_wake, Some(at(2024, 3, 1, 0)));
}

/// Rules: R-013
#[test]
fn a_cancellation_before_the_checkout_settled_bills_nothing() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.start.settled = None;
    facts.start.checkout = true;
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 3_100));
    facts.ending = Some(Ending {
        at: at(2024, 1, 5, 12),
        kind: EndingKind::Cancel,
    });
    let evaluation = run(&facts, at(2024, 1, 10, 0));
    assert!(evaluation.charges.is_empty());
    assert_eq!(evaluation.state.status, Status::Cancelled);
}

/// A cancellation on Mar 20 still to come leaves a pending checkout its opening invoice: January
/// whole, 3100, on the invoice of Jan 1, and nothing after it until the start settles.
/// Rules: R-013
#[test]
fn a_cancellation_still_to_come_leaves_a_pending_checkout_its_opening_invoice() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.start.settled = None;
    facts.start.checkout = true;
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 3_100));
    facts.ending = Some(Ending {
        at: at(2024, 3, 20, 0),
        kind: EndingKind::Cancel,
    });
    let pending = run(&facts, at(2024, 1, 10, 0));
    assert_eq!(pending.state.status, Status::Pending);
    assert!(pending.state.pending_checkout);
    let charges: Vec<_> = pending
        .charges
        .iter()
        .map(|c| (c.key.from, c.key.until, c.amount, c.document))
        .collect();
    assert_eq!(
        charges,
        vec![(
            d(2024, 1, 1),
            d(2024, 2, 1),
            3_100,
            recurring(d(2024, 1, 1))
        )]
    );

    // Still unsettled once the cancellation applies: it never started, nothing is billed.
    let ended = run(&facts, at(2024, 3, 21, 0));
    assert!(ended.charges.is_empty());
    assert_eq!(ended.state.status, Status::Cancelled);
}

/// A cancellation on Jan 20 still to come leaves the current period January's, as stored: it ends
/// the subscription on its date, and the charges stop there. Once it applies, the period ends.
/// Rules: R-016
#[test]
fn a_cancellation_still_to_come_leaves_the_current_period_whole() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 3_100));
    facts.ending = Some(Ending {
        at: at(2024, 1, 20, 0),
        kind: EndingKind::Cancel,
    });
    let before = run(&facts, at(2024, 1, 10, 0));
    assert_eq!(before.state.status, Status::Active);
    assert_eq!(
        before.state.current_period,
        Some((d(2024, 1, 1), Some(d(2024, 2, 1))))
    );
    assert_eq!(before.next_wake, Some(at(2024, 1, 20, 0)));
    let after = run(&facts, at(2024, 1, 21, 0));
    assert_eq!(after.state.status, Status::Cancelled);
    assert_eq!(after.state.current_period, Some((d(2024, 1, 20), None)));
}

/// A fixed term ending on Jan 20 cuts the period it ends in, as its renewal stores it.
/// Rules: R-084
#[test]
fn a_fixed_term_cuts_the_current_period_at_its_end() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 3_100));
    facts.ending = Some(Ending {
        at: at(2024, 1, 20, 0),
        kind: EndingKind::Complete,
    });
    let before = run(&facts, at(2024, 1, 10, 0));
    assert_eq!(
        before.state.current_period,
        Some((d(2024, 1, 1), Some(d(2024, 1, 20))))
    );
}

/// A monthly plan switched to a yearly one on July 1, the change's origin: six monthly
/// periods, then a year from July 1.
/// Rules: R-084
#[test]
fn a_scheduled_plan_window_change_moves_the_grid_at_its_origin() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts
        .plan_windows
        .push(plan(101, d(2024, 7, 1), 12, d(2024, 7, 1)));
    facts.fees.push(FeeFact {
        to: Some(d(2024, 7, 1)),
        ..fixed(10, 1, d(2024, 1, 1), 1, 1_000)
    });
    facts.fees.push(fixed(20, 2, d(2024, 7, 1), 12, 12_000));
    let evaluation = run(&facts, at(2024, 8, 15, 0));
    assert_eq!(of(&evaluation, 1).len(), 6);
    let yearly = of(&evaluation, 2);
    assert_eq!(periods(&yearly), vec![(d(2024, 7, 1), d(2025, 7, 1))]);
    assert_eq!(yearly[0].amount, 12_000);
    assert_eq!(yearly[0].document, recurring(d(2024, 7, 1)));
    assert_eq!(evaluation.state.plan_version, Some(id(101)));
    assert_eq!(evaluation.state.cadence, Some(12));
    assert_eq!(
        evaluation.state.current_period,
        Some((d(2024, 7, 1), Some(d(2025, 7, 1))))
    );
    assert_eq!(evaluation.next_wake, Some(at(2025, 7, 1, 0)));
}

/// Switched to yearly on March 15, its origin the next renewal on April 1: the old fee stops on
/// the 15th, the new one bills its 17 days of the year ending April 1 on an adjustment invoice,
/// then a year from April 1. The period holding March 20 is still March's.
/// Rules: R-084, R-083
#[test]
fn a_change_of_cadence_mid_period_bills_its_stub_on_the_new_grid() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts
        .plan_windows
        .push(plan(101, d(2024, 3, 15), 12, d(2024, 4, 1)));
    facts.fees.push(FeeFact {
        to: Some(d(2024, 3, 15)),
        ..fixed(10, 1, d(2024, 1, 1), 1, 1_000)
    });
    facts.fees.push(fixed(20, 2, d(2024, 3, 15), 12, 12_000));

    let mid = run(&facts, at(2024, 3, 20, 0));
    assert_eq!(
        mid.state.current_period,
        Some((d(2024, 3, 1), Some(d(2024, 4, 1))))
    );

    let evaluation = run(&facts, at(2024, 4, 10, 0));
    let old = of(&evaluation, 1);
    assert_eq!(old[2].window, (d(2024, 3, 1), d(2024, 3, 15)));
    assert_eq!(old[2].amount, 452);
    let new = of(&evaluation, 2);
    assert_eq!(
        periods(&new),
        vec![
            (d(2023, 4, 1), d(2024, 4, 1)),
            (d(2024, 4, 1), d(2025, 4, 1))
        ]
    );
    assert_eq!(new[0].window, (d(2024, 3, 15), d(2024, 4, 1)));
    assert_eq!(new[0].amount, 557);
    assert_eq!(
        new[0].document,
        DocumentSlot::Adjustment { on: d(2024, 3, 15) }
    );
    assert_eq!(new[1].amount, 12_000);
    assert_eq!(new[1].document, recurring(d(2024, 4, 1)));
}

/// A yearly fee of 120.00 amended to 10.00 a month on March 15, its lineage kept, the plan moving
/// onto the monthly grid: the year bills 12000 less what is left of it at the 15th (12000 ×
/// 292/366 = 9574), so 2426; the monthly run starts on its own grid, its stub to April 1 priced
/// and keyed over March, 1000 × 17/31 = 548, then 1000 on each renewal under its own month.
/// Rules: R-079, R-084
#[test]
fn a_fee_changing_cadence_on_its_lineage_bills_the_new_cadence_on_its_own_periods() {
    let mut facts = subscription(d(2024, 1, 1), 1, 12);
    facts
        .plan_windows
        .push(plan(101, d(2024, 3, 15), 1, d(2024, 1, 1)));
    facts.fees.push(FeeFact {
        to: Some(d(2024, 3, 15)),
        ..fixed(10, 1, d(2024, 1, 1), 12, 12_000)
    });
    facts.fees.push(fixed(20, 1, d(2024, 3, 15), 1, 1_000));
    let evaluation = run(&facts, at(2025, 1, 10, 0));
    let year = (d(2024, 1, 1), d(2025, 1, 1));
    let mut expected = vec![
        (
            year,
            (d(2024, 1, 1), d(2024, 3, 15)),
            2_426,
            recurring(d(2024, 1, 1)),
        ),
        (
            (d(2024, 3, 1), d(2024, 4, 1)),
            (d(2024, 3, 15), d(2024, 4, 1)),
            548,
            DocumentSlot::Adjustment { on: d(2024, 3, 15) },
        ),
    ];
    for month in 4..=12 {
        let until = d(2024, month, 1)
            .checked_add_months(Months::new(1))
            .unwrap();
        expected.push((
            (d(2024, month, 1), until),
            (d(2024, month, 1), until),
            1_000,
            recurring(d(2024, month, 1)),
        ));
    }
    expected.push((
        (d(2025, 1, 1), d(2025, 2, 1)),
        (d(2025, 1, 1), d(2025, 2, 1)),
        1_000,
        recurring(d(2025, 1, 1)),
    ));
    let charges: Vec<_> = of(&evaluation, 1)
        .iter()
        .map(|c| ((c.key.from, c.key.until), c.window, c.amount, c.document))
        .collect();
    assert_eq!(charges, expected);
}

/// 29.00 a month from Jan 1, 2025, amended to 500.00 a year on Feb 16 and to 49.99 a month from
/// Apr 1, its lineage kept. The year's stub, 50000 × 319/365 − 50000 × 275/365 = 6028 of the
/// 43699 billed, is a run of its own; April is the monthly run's own charge, 4999. With the
/// year's line voided, its void forgives the year alone and April bills 4999; keyed under the
/// year, April fell under the void and billed nothing.
/// Rules: R-084, R-096
#[test]
fn a_run_of_another_cadence_on_a_lineage_is_no_part_of_a_voided_period() {
    let mut facts = subscription(d(2025, 1, 1), 1, 1);
    facts.fees.push(FeeFact {
        to: Some(d(2025, 2, 16)),
        ..fixed(10, 1, d(2025, 1, 1), 1, 2_900)
    });
    facts.fees.push(FeeFact {
        to: Some(d(2025, 4, 1)),
        ..fixed(11, 1, d(2025, 2, 16), 12, 50_000)
    });
    facts.fees.push(fixed(12, 1, d(2025, 4, 1), 1, 4_999));
    let evaluation = run(&facts, at(2025, 4, 2, 0));
    let year = ChargeKey {
        subscription: Some(id(SUBSCRIPTION)),
        fee: lineage(1),
        from: d(2025, 1, 1),
        until: d(2026, 1, 1),
        kind: ChargeKind::Fee,
        denomination: Denomination::Money,
    };
    let april = ChargeKey {
        from: d(2025, 4, 1),
        until: d(2025, 5, 1),
        ..year.clone()
    };
    let expected: Vec<Expected> = evaluation
        .expected()
        .into_iter()
        .filter(|e| e.key == year || e.key == april)
        .collect();
    assert_eq!(
        expected,
        vec![
            Expected {
                key: year.clone(),
                amount: 6_028
            },
            Expected {
                key: april.clone(),
                amount: 4_999
            },
        ]
    );
    let mut voided = line("year".to_owned(), &year, 43_699);
    voided.from = d(2025, 2, 16);
    voided.voided = true;
    assert_eq!(
        reconcile(&expected, &[voided], TakeBack::LatestFirst),
        vec![Adjustment::Bill {
            key: april,
            amount: 4_999
        }]
    );
}

/// 500.00 a month in arrears from Jan 2, 2025, changed on Apr 5 to 29.00 a year in advance on
/// its lineage (origin May 2), amended on Apr 10 to 49.99 a year, cancelled from May 5. The
/// stubs to May 2, 215 and 126, are the year's: April's arrears charge bills none of their days.
/// The cancellation gives back the year from May 2 alone, 4999 − 4999 × 3/365 = 4958; read as
/// April's, the narrower period, the 126 stub of a year fully used came back too.
/// Rules: R-077, R-080, R-084
#[test]
fn a_line_counts_for_the_charge_billing_its_days_not_the_narrowest_holding_it() {
    let mut facts = subscription(d(2025, 1, 2), 2, 1);
    facts
        .plan_windows
        .push(plan(101, d(2025, 4, 5), 12, d(2025, 5, 2)));
    facts.fees.push(FeeFact {
        to: Some(d(2025, 4, 5)),
        billing: Billing::Arrears,
        ..fixed(10, 1, d(2025, 1, 2), 1, 50_000)
    });
    facts.fees.push(FeeFact {
        to: Some(d(2025, 4, 10)),
        ..fixed(11, 1, d(2025, 4, 5), 12, 2_900)
    });
    facts.fees.push(fixed(12, 1, d(2025, 4, 10), 12, 4_999));
    facts.ending = Some(Ending {
        at: midnight(d(2025, 5, 5)),
        kind: EndingKind::Cancel,
    });
    let evaluation = run(&facts, at(2025, 5, 5, 1));
    let key = |from: NaiveDate, until: NaiveDate| ChargeKey {
        subscription: Some(id(SUBSCRIPTION)),
        fee: lineage(1),
        from,
        until,
        kind: ChargeKind::Fee,
        denomination: Denomination::Money,
    };
    let (april, stub_year, year) = (
        key(d(2025, 4, 2), d(2025, 5, 2)),
        key(d(2024, 5, 2), d(2025, 5, 2)),
        key(d(2025, 5, 2), d(2026, 5, 2)),
    );
    let stub = |id: &str, from: NaiveDate, subtotal: i64| IssuedLine {
        from,
        until: d(2025, 5, 2),
        ..line(id.to_owned(), &stub_year, subtotal)
    };
    let issued = [
        line(
            "april".to_owned(),
            &key(d(2025, 4, 2), d(2025, 4, 5)),
            5_000,
        ),
        stub("change", d(2025, 4, 5), 215),
        stub("amendment", d(2025, 4, 10), 126),
        line("year".to_owned(), &year, 4_999),
    ];
    let expected: Vec<Expected> = evaluation
        .expected()
        .into_iter()
        .filter(|e| [&april, &stub_year, &year].contains(&&e.key))
        .collect();
    assert_eq!(
        expected.iter().map(|e| e.amount).collect::<Vec<_>>(),
        vec![341, 5_000, 41]
    );
    let keys: Vec<&ChargeKey> = expected.iter().map(|e| &e.key).collect();
    let mut adjustments = Vec::new();
    for charge in &expected {
        let owned: Vec<IssuedLine> = issued
            .iter()
            .filter(|l| owner(l, keys.iter().copied(), &evaluation.charges) == Some(&charge.key))
            .cloned()
            .collect();
        adjustments.extend(reconcile(
            std::slice::from_ref(charge),
            &owned,
            TakeBack::Unused(d(2025, 5, 5)),
        ));
    }
    assert_eq!(
        adjustments,
        vec![Adjustment::Credit {
            line: "year".to_owned(),
            amount: 4_958
        }]
    );
}

/// 500.00 a month in arrears from Jan 2, 2025, changed on Jan 4 to 500.00 a year on its lineage
/// (origin Feb 2), amended on Jan 8 to 29.00 a month. The year's stub billed on Jan 4, 3962 for
/// [Jan 4, Feb 2), counts for the year that started it, though January's month holds it and
/// bills days of it: the year keeps 50000 × 29/366 − 50000 × 25/366 = 547 and gives back 3415,
/// and the month bills its stub to Feb 2, 2900 × 2/31 = 187, beside January's arrears.
/// Rules: R-080, R-084
#[test]
fn a_line_counts_for_the_charge_that_started_it() {
    let mut facts = subscription(d(2025, 1, 2), 2, 1);
    facts
        .plan_windows
        .push(plan(101, d(2025, 1, 4), 12, d(2025, 2, 2)));
    facts.fees.push(FeeFact {
        to: Some(d(2025, 1, 4)),
        billing: Billing::Arrears,
        ..fixed(10, 1, d(2025, 1, 2), 1, 50_000)
    });
    facts.fees.push(FeeFact {
        to: Some(d(2025, 1, 8)),
        ..fixed(11, 1, d(2025, 1, 4), 12, 50_000)
    });
    facts.fees.push(FeeFact {
        to: Some(d(2025, 1, 10)),
        ..fixed(12, 1, d(2025, 1, 8), 1, 2_900)
    });
    let evaluation = run(&facts, at(2025, 1, 8, 1));
    let key = |from: NaiveDate, until: NaiveDate| ChargeKey {
        subscription: Some(id(SUBSCRIPTION)),
        fee: lineage(1),
        from,
        until,
        kind: ChargeKind::Fee,
        denomination: Denomination::Money,
    };
    let (month, year) = (
        key(d(2025, 1, 2), d(2025, 2, 2)),
        key(d(2024, 2, 2), d(2025, 2, 2)),
    );
    let stub = IssuedLine {
        from: d(2025, 1, 4),
        ..line("stub".to_owned(), &year, 3_962)
    };
    let expected: Vec<Expected> = evaluation
        .expected()
        .into_iter()
        .filter(|e| e.key == month || e.key == year)
        .collect();
    let keys: Vec<&ChargeKey> = expected.iter().map(|e| &e.key).collect();
    assert_eq!(
        owner(&stub, keys.iter().copied(), &evaluation.charges),
        Some(&year)
    );
    let amounts: Vec<(&ChargeKey, i64)> = expected.iter().map(|e| (&e.key, e.amount)).collect();
    assert_eq!(amounts, vec![(&year, 547), (&month, 3_226 + 187)]);
    let year_charge = expected.iter().find(|e| e.key == year).unwrap();
    assert_eq!(
        reconcile(
            std::slice::from_ref(year_charge),
            &[stub],
            TakeBack::Unused(d(2025, 1, 8))
        ),
        vec![Adjustment::Credit {
            line: "stub".to_owned(),
            amount: 3_415
        }]
    );
}

/// A monthly fee of 10.00 amended to 120.00 a year on March 15, its lineage kept, the year's
/// grid from April 1: March bills 1000 less 1000 × 17/31 = 548, so 452; the stub to April 1 is
/// priced over the year it falls in, 12000 × 17/366 = 557, under that year's key; then the year.
/// Rules: R-079, R-084
#[test]
fn a_fee_changing_to_a_longer_cadence_on_its_lineage_prices_its_stub_over_its_own_year() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts
        .plan_windows
        .push(plan(101, d(2024, 3, 15), 12, d(2024, 4, 1)));
    facts.fees.push(FeeFact {
        to: Some(d(2024, 3, 15)),
        ..fixed(10, 1, d(2024, 1, 1), 1, 1_000)
    });
    facts.fees.push(fixed(20, 1, d(2024, 3, 15), 12, 12_000));
    let evaluation = run(&facts, at(2024, 4, 10, 0));
    let march = (d(2024, 3, 1), d(2024, 4, 1));
    let charges: Vec<_> = of(&evaluation, 1)
        .iter()
        .map(|c| ((c.key.from, c.key.until), c.window, c.amount, c.document))
        .collect();
    assert_eq!(
        charges,
        vec![
            (
                (d(2024, 1, 1), d(2024, 2, 1)),
                (d(2024, 1, 1), d(2024, 2, 1)),
                1_000,
                recurring(d(2024, 1, 1))
            ),
            (
                (d(2024, 2, 1), d(2024, 3, 1)),
                (d(2024, 2, 1), d(2024, 3, 1)),
                1_000,
                recurring(d(2024, 2, 1))
            ),
            (
                march,
                (d(2024, 3, 1), d(2024, 3, 15)),
                452,
                recurring(d(2024, 3, 1))
            ),
            (
                (d(2023, 4, 1), d(2024, 4, 1)),
                (d(2024, 3, 15), d(2024, 4, 1)),
                557,
                DocumentSlot::Adjustment { on: d(2024, 3, 15) }
            ),
            (
                (d(2024, 4, 1), d(2025, 4, 1)),
                (d(2024, 4, 1), d(2025, 4, 1)),
                12_000,
                recurring(d(2024, 4, 1))
            ),
        ]
    );
}

/// A yearly 120.00 add-on bought Jan 1 on a monthly plan, the plan moving to quarterly on March
/// 15 (origin April 1): the add-on keeps its year, billed nor given back at the change, and renews
/// Jan 1; the quarterly fee the change writes bills 3000 × 17/91 = 560, then quarters from April 1.
/// Rules: R-084
#[test]
fn a_fee_kept_through_a_change_of_cadence_keeps_its_grid() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts
        .plan_windows
        .push(plan(101, d(2024, 3, 15), 3, d(2024, 4, 1)));
    facts.fees.push(FeeFact {
        to: Some(d(2024, 3, 15)),
        ..fixed(10, 1, d(2024, 1, 1), 1, 1_000)
    });
    facts.fees.push(fixed(20, 2, d(2024, 3, 15), 3, 3_000));
    let mut add_on = fixed(30, 3, d(2024, 1, 1), 12, 12_000);
    add_on.lineage = Fee::AddOn(id(3));
    add_on.plan_component = false;
    facts.fees.push(add_on);
    let evaluation = run(&facts, at(2025, 1, 10, 0));
    let charges = |fee: &Fee| -> Vec<_> {
        evaluation
            .charges
            .iter()
            .filter(|c| c.key.fee == *fee)
            .map(|c| ((c.key.from, c.key.until), c.window, c.amount, c.document))
            .collect()
    };
    assert_eq!(
        charges(&Fee::AddOn(id(3))),
        vec![
            (
                (d(2024, 1, 1), d(2025, 1, 1)),
                (d(2024, 1, 1), d(2025, 1, 1)),
                12_000,
                recurring(d(2024, 1, 1))
            ),
            (
                (d(2025, 1, 1), d(2026, 1, 1)),
                (d(2025, 1, 1), d(2026, 1, 1)),
                12_000,
                recurring(d(2025, 1, 1))
            ),
        ]
    );
    let quarter =
        |from: NaiveDate, until: NaiveDate| ((from, until), (from, until), 3_000, recurring(from));
    assert_eq!(
        charges(&lineage(2)),
        vec![
            (
                (d(2024, 1, 1), d(2024, 4, 1)),
                (d(2024, 3, 15), d(2024, 4, 1)),
                560,
                DocumentSlot::Adjustment { on: d(2024, 3, 15) }
            ),
            quarter(d(2024, 4, 1), d(2024, 7, 1)),
            quarter(d(2024, 7, 1), d(2024, 10, 1)),
            quarter(d(2024, 10, 1), d(2025, 1, 1)),
            quarter(d(2025, 1, 1), d(2025, 4, 1)),
        ]
    );
}

/// Usage at 0.10 a unit, yearly from Feb 15, 2023, then monthly from Feb 16, 2024 (origin Mar
/// 15), then quarterly from Mar 16 (origin Apr 15), cancelled on Apr 15 at 01:00. The quarter's
/// window [Mar 16, Apr 15) ends on its own grid's boundary, though keyed on the year it falls in:
/// it reads to Apr 15's midnight, 30 units, 300; the hour of Apr 15 is read once, 1 unit, 10.
/// Rules: R-016, R-084
#[test]
fn an_ending_on_its_own_boundary_reads_the_window_keyed_on_a_longer_period_to_midnight() {
    let mut facts = subscription(d(2023, 2, 15), 15, 12);
    facts
        .plan_windows
        .push(plan(101, d(2024, 2, 16), 1, d(2024, 3, 15)));
    facts
        .plan_windows
        .push(plan(102, d(2024, 3, 16), 3, d(2024, 4, 15)));
    facts.fees.push(FeeFact {
        to: Some(d(2024, 2, 16)),
        ..metered(10, 1, d(2023, 2, 15), 12, 10)
    });
    facts.fees.push(FeeFact {
        to: Some(d(2024, 3, 16)),
        ..metered(20, 1, d(2024, 2, 16), 1, 10)
    });
    facts.fees.push(metered(30, 1, d(2024, 3, 16), 3, 10));
    facts.ending = Some(Ending {
        at: at(2024, 4, 15, 1),
        kind: EndingKind::Cancel,
    });
    let usage = reads(&[
        (read(at(2024, 3, 16, 0), at(2024, 4, 15, 0)), 30),
        (read(at(2024, 3, 16, 0), at(2024, 4, 15, 1)), 31),
        (read(at(2024, 4, 15, 0), at(2024, 4, 15, 1)), 1),
    ]);
    let evaluation = evaluate(&facts, at(2024, 4, 15, 1), Some(&usage), &TestPricer).unwrap();
    let charges: Vec<_> = of(&evaluation, 1)
        .iter()
        .filter(|c| c.key.from >= d(2024, 3, 16))
        .map(|c| ((c.key.from, c.key.until), c.usage, c.amount, c.document))
        .collect();
    assert_eq!(
        charges,
        vec![
            (
                (d(2024, 3, 16), d(2024, 4, 15)),
                Some(UsageWindow {
                    from: at(2024, 3, 16, 0),
                    until: at(2024, 4, 15, 0)
                }),
                300,
                recurring(d(2024, 4, 15))
            ),
            (
                (d(2024, 4, 15), d(2024, 4, 15)),
                Some(UsageWindow {
                    from: at(2024, 4, 15, 0),
                    until: at(2024, 4, 15, 1)
                }),
                10,
                DocumentSlot::Final
            ),
        ]
    );
}

/// Usage at 0.10 a unit from Jan 1, a second row over it from Jan 2 with the first left open,
/// cancelled on Jan 2 at 02:00: Jan 1 is the first row's, 24 units, 240; Jan 2's two hours the
/// second row's, read once on the final usage, 2 units, 20.
/// Rules: R-016
#[test]
fn a_row_taking_over_on_the_ending_day_reads_that_morning_once() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(metered(10, 1, d(2024, 1, 1), 1, 10));
    facts.fees.push(metered(20, 1, d(2024, 1, 2), 1, 10));
    facts.ending = Some(Ending {
        at: at(2024, 1, 2, 2),
        kind: EndingKind::Cancel,
    });
    let usage = reads(&[
        (read(at(2024, 1, 1, 0), at(2024, 1, 2, 0)), 24),
        (read(at(2024, 1, 1, 0), at(2024, 1, 2, 2)), 26),
        (read(at(2024, 1, 2, 0), at(2024, 1, 2, 2)), 2),
    ]);
    let evaluation = evaluate(&facts, at(2024, 1, 2, 2), Some(&usage), &TestPricer).unwrap();
    let charges: Vec<_> = of(&evaluation, 1)
        .iter()
        .map(|c| ((c.key.from, c.key.until), c.usage, c.amount))
        .collect();
    assert_eq!(
        charges,
        vec![
            (
                (d(2024, 1, 1), d(2024, 1, 2)),
                Some(UsageWindow {
                    from: at(2024, 1, 1, 0),
                    until: at(2024, 1, 2, 0)
                }),
                240
            ),
            (
                (d(2024, 1, 2), d(2024, 1, 2)),
                Some(UsageWindow {
                    from: at(2024, 1, 2, 0),
                    until: at(2024, 1, 2, 2)
                }),
                20
            ),
        ]
    );
}

/// Usage at 0.10 a unit from Jan 1, a row at 0.20 over it from Jan 10 until Jan 20, cancelled on
/// Jan 20 at 02:00: Jan 10 to 20 is the second row's, 100 units, 2000; Jan 20's two hours go back
/// to the first row, read on the final usage, 2 units, 20.
/// Rules: R-016
#[test]
fn a_row_resuming_on_the_ending_day_bills_that_morning() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(metered(10, 1, d(2024, 1, 1), 1, 10));
    facts.fees.push(FeeFact {
        to: Some(d(2024, 1, 20)),
        ..metered(20, 1, d(2024, 1, 10), 1, 20)
    });
    facts.ending = Some(Ending {
        at: at(2024, 1, 20, 2),
        kind: EndingKind::Cancel,
    });
    let usage = reads(&[
        (read(at(2024, 1, 1, 0), at(2024, 1, 10, 0)), 90),
        (read(at(2024, 1, 10, 0), at(2024, 1, 20, 0)), 100),
        (read(at(2024, 1, 20, 0), at(2024, 1, 20, 2)), 2),
    ]);
    let evaluation = evaluate(&facts, at(2024, 1, 20, 2), Some(&usage), &TestPricer).unwrap();
    let charges: Vec<_> = of(&evaluation, 1)
        .iter()
        .map(|c| ((c.key.from, c.key.until), c.usage, c.amount))
        .collect();
    assert_eq!(
        charges,
        vec![
            (
                (d(2024, 1, 1), d(2024, 1, 10)),
                Some(UsageWindow {
                    from: at(2024, 1, 1, 0),
                    until: at(2024, 1, 10, 0)
                }),
                900
            ),
            (
                (d(2024, 1, 10), d(2024, 1, 20)),
                Some(UsageWindow {
                    from: at(2024, 1, 10, 0),
                    until: at(2024, 1, 20, 0)
                }),
                2_000
            ),
            (
                (d(2024, 1, 20), d(2024, 1, 20)),
                Some(UsageWindow {
                    from: at(2024, 1, 20, 0),
                    until: at(2024, 1, 20, 2)
                }),
                20
            ),
        ]
    );
}

/// Usage at 0.10 a unit, monthly from Jan 1, then quarterly on its lineage from Jan 2, cancelled
/// on Feb 1 at 02:00: Feb 1 is no boundary of its quarter [Jan 1, Apr 1), so the window from Jan
/// 2, keyed on January, reads to the ending's instant, 30 units and the two hours, 320.
/// Rules: R-016, R-084
#[test]
fn an_ending_inside_its_own_grid_period_reads_to_the_instant_though_keyed_on_a_shorter_one() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(FeeFact {
        to: Some(d(2024, 1, 2)),
        ..metered(10, 1, d(2024, 1, 1), 1, 10)
    });
    facts.fees.push(metered(20, 1, d(2024, 1, 2), 3, 10));
    facts.ending = Some(Ending {
        at: at(2024, 2, 1, 2),
        kind: EndingKind::Cancel,
    });
    let usage = reads(&[
        (read(at(2024, 1, 1, 0), at(2024, 1, 2, 0)), 1),
        (read(at(2024, 1, 2, 0), at(2024, 2, 1, 0)), 30),
        (read(at(2024, 1, 2, 0), at(2024, 2, 1, 2)), 32),
        (read(at(2024, 2, 1, 0), at(2024, 2, 1, 2)), 2),
    ]);
    let evaluation = evaluate(&facts, at(2024, 2, 1, 2), Some(&usage), &TestPricer).unwrap();
    let charges: Vec<_> = of(&evaluation, 1)
        .iter()
        .map(|c| ((c.key.from, c.key.until), c.usage, c.amount))
        .collect();
    assert_eq!(
        charges,
        vec![
            (
                (d(2024, 1, 1), d(2024, 1, 2)),
                Some(UsageWindow {
                    from: at(2024, 1, 1, 0),
                    until: at(2024, 1, 2, 0)
                }),
                10
            ),
            (
                (d(2024, 1, 2), d(2024, 2, 1)),
                Some(UsageWindow {
                    from: at(2024, 1, 2, 0),
                    until: at(2024, 2, 1, 2)
                }),
                320
            ),
        ]
    );
}

/// 29.00 a year from Jan 13, 2025, amended to 29.00 a month on Jan 15 and to 123.45 a year on
/// Jan 18, each amendment moving the subscription's cadence, its grid kept. The years bill under
/// the year: 2900 − 2900 × 363/365 = 16, and from Jan 18 12345 × 360/365 = 12176; the month's 3
/// days under its month, over January, 2900 × 29/31 − 2900 × 26/31 = 2713 − 2432 = 281.
/// Rules: R-084, R-079
#[test]
fn a_year_after_a_month_nested_in_it_is_priced_over_the_year() {
    let mut facts = subscription(d(2025, 1, 13), 13, 12);
    facts.plan_windows[0].to = Some(d(2025, 1, 15));
    facts.plan_windows.push(PlanWindow {
        to: Some(d(2025, 1, 18)),
        ..plan(100, d(2025, 1, 15), 1, d(2025, 1, 13))
    });
    facts
        .plan_windows
        .push(plan(100, d(2025, 1, 18), 12, d(2025, 1, 13)));
    facts.fees.push(FeeFact {
        to: Some(d(2025, 1, 15)),
        ..fixed(10, 1, d(2025, 1, 13), 12, 2_900)
    });
    facts.fees.push(FeeFact {
        to: Some(d(2025, 1, 18)),
        ..fixed(11, 1, d(2025, 1, 15), 1, 2_900)
    });
    facts.fees.push(fixed(12, 1, d(2025, 1, 18), 12, 12_345));
    let evaluation = run(&facts, at(2025, 1, 20, 0));
    let year = (d(2025, 1, 13), d(2026, 1, 13));
    let charges: Vec<_> = of(&evaluation, 1)
        .iter()
        .map(|c| ((c.key.from, c.key.until), c.window, c.amount))
        .collect();
    assert_eq!(
        charges,
        vec![
            (year, (d(2025, 1, 13), d(2025, 1, 15)), 16),
            (
                (d(2025, 1, 13), d(2025, 2, 13)),
                (d(2025, 1, 15), d(2025, 1, 18)),
                281
            ),
            (year, (d(2025, 1, 18), d(2026, 1, 13)), 12_176),
        ]
    );
}

/// A half-yearly subscription switched to monthly on November 14: it is then in November, the
/// new grid's month inside the half-year that started June 1.
/// Rules: R-084
#[test]
fn a_change_to_a_shorter_cadence_moves_the_current_period_onto_the_new_grid() {
    let mut facts = subscription(d(2024, 6, 1), 1, 6);
    facts
        .plan_windows
        .push(plan(101, d(2024, 11, 14), 1, d(2024, 6, 1)));
    facts.fees.push(fixed(10, 1, d(2024, 6, 1), 6, 6_000));
    let evaluation = run(&facts, at(2024, 11, 20, 0));
    assert_eq!(
        evaluation.state.current_period,
        Some((d(2024, 11, 1), Some(d(2024, 12, 1))))
    );
}

fn seats(quantities: &[(NaiveDate, u64)]) -> Facts<TestFee> {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 100));
    facts.seats = quantities
        .iter()
        .enumerate()
        .map(|(n, (on, quantity))| SeatStep {
            id: id(500 + n as u128),
            lineage: lineage(1),
            at: *on,
            quantity: *quantity,
            removed_at_renewal: 0,
        })
        .collect();
    facts
}

fn issue(charges: &[Expected]) -> Vec<IssuedLine> {
    charges
        .iter()
        .enumerate()
        .filter(|(_, charge)| charge.amount > 0)
        .map(|(n, charge)| line(format!("l{n}"), &charge.key, charge.amount))
        .collect()
}

fn line(id: String, key: &ChargeKey, subtotal: i64) -> IssuedLine {
    IssuedLine {
        id,
        key: key.clone(),
        kind: IssuedKind::Fee {
            price_component: None,
            add_on: false,
        },
        lineage: None,
        quantity: None,
        unit_priced: false,
        from: key.from,
        until: key.until,
        subtotal,
        quoted: None,
        already_credited: 0,
        corrected: 0,
        settled: None,
        voided: false,
    }
}

/// Applies `adjustments` to `lines` as the documents would: a bill is a new line, a credit a note
/// on its line.
fn apply(lines: &mut Vec<IssuedLine>, adjustments: &[Adjustment]) {
    for adjustment in adjustments {
        match adjustment {
            Adjustment::Bill { key, amount } => {
                let n = lines.len();
                lines.push(line(format!("b{n}"), key, *amount));
            }
            Adjustment::Credit { line, amount } => {
                let line = lines.iter_mut().find(|l| l.id == *line).unwrap();
                line.already_credited += amount;
                line.corrected += amount;
            }
            Adjustment::Short { .. } => {}
        }
    }
}

/// Ten seats from January 1, fifteen from the 16th: the period's price at its start, then the
/// difference of the 16 days left on an adjustment invoice; together the terms tiling January.
/// Rules: R-079, R-080
#[test]
fn a_seat_step_bills_its_difference_when_it_takes_effect() {
    let facts = seats(&[(d(2024, 1, 1), 10), (d(2024, 1, 16), 15)]);
    let evaluation = run(&facts, at(2024, 1, 20, 0));
    let charges = of(&evaluation, 1);
    assert_eq!(charges.len(), 2);
    assert_eq!(
        (charges[0].amount, charges[0].document),
        (1_000, recurring(d(2024, 1, 1)))
    );
    assert_eq!(
        (charges[1].amount, charges[1].document),
        (258, DocumentSlot::Adjustment { on: d(2024, 1, 16) })
    );
    assert_eq!(charges[1].quantity, Some(Decimal::from(15)));
    assert_eq!(charges[1].window, (d(2024, 1, 16), d(2024, 2, 1)));
    let term = |full, from, until| Term {
        fee: lineage(1),
        full,
        period: (d(2024, 1, 1), d(2024, 2, 1)),
        from,
        until,
    };
    let tiled = crate::reconcile::expected(
        Some(id(SUBSCRIPTION)),
        &[
            term(1_000, d(2024, 1, 1), d(2024, 1, 16)),
            term(1_500, d(2024, 1, 16), d(2024, 2, 1)),
        ],
    );
    assert_eq!(evaluation.expected(), tiled);

    let before = run(&facts, at(2024, 1, 10, 0));
    assert_eq!(of(&before, 1).len(), 1);
    assert_eq!(before.next_wake, Some(at(2024, 1, 16, 0)));
    let mut lines = issue(&before.expected());
    let adjustments = reconcile(&evaluation.expected(), &lines, TakeBack::LatestFirst);
    assert_eq!(
        adjustments,
        vec![Adjustment::Bill {
            key: charges[0].key.clone(),
            amount: 258
        }]
    );
    apply(&mut lines, &adjustments);
    assert!(reconcile(&evaluation.expected(), &lines, TakeBack::LatestFirst).is_empty());
}

/// Rules: R-080
#[test]
fn a_seat_step_down_takes_back_what_is_left_of_the_period() {
    let facts = seats(&[(d(2024, 1, 1), 10), (d(2024, 1, 16), 5)]);
    let mut lines = issue(&run(&facts, at(2024, 1, 10, 0)).expected());
    let evaluation = run(&facts, at(2024, 1, 20, 0));
    assert_eq!(of(&evaluation, 1)[1].amount, -258);
    let adjustments = reconcile(&evaluation.expected(), &lines, TakeBack::LatestFirst);
    assert_eq!(
        adjustments,
        vec![Adjustment::Credit {
            line: "l0".to_owned(),
            amount: 258
        }]
    );
    apply(&mut lines, &adjustments);
    assert!(reconcile(&evaluation.expected(), &lines, TakeBack::LatestFirst).is_empty());
}

/// Ten seats, five removed for January 16, the period's end when they were removed, which a
/// change moved to February 1: the removal waits for that renewal and gives nothing back.
/// Rules: R-108
#[test]
fn a_removal_for_a_period_end_that_moved_waits_for_the_next_renewal() {
    let mut facts = seats(&[(d(2024, 1, 1), 10), (d(2024, 1, 16), 5)]);
    facts.seats[1].removed_at_renewal = 5;
    let january = run(&facts, at(2024, 1, 20, 0));
    let charges = of(&january, 1);
    assert_eq!(
        charges
            .iter()
            .map(|charge| (charge.amount, charge.document))
            .collect::<Vec<_>>(),
        vec![(1_000, recurring(d(2024, 1, 1)))]
    );
    let lines = issue(&january.expected());
    let february = run(&facts, at(2024, 2, 1, 0));
    assert_eq!(
        of(&february, 1)
            .iter()
            .map(|charge| (charge.amount, charge.quantity, charge.document))
            .collect::<Vec<_>>(),
        vec![
            (1_000, Some(Decimal::from(10)), recurring(d(2024, 1, 1))),
            (500, Some(Decimal::from(5)), recurring(d(2024, 2, 1))),
        ]
    );
    assert_eq!(
        reconcile(&january.expected(), &lines, TakeBack::LatestFirst),
        vec![]
    );

    // On a renewal, as scheduled, it is that renewal's count.
    let mut on_time = seats(&[(d(2024, 1, 1), 10), (d(2024, 2, 1), 5)]);
    on_time.seats[1].removed_at_renewal = 5;
    let mut plain = on_time.clone();
    plain.seats[1].removed_at_renewal = 0;
    assert_eq!(
        run(&on_time, at(2024, 2, 1, 0)).charges,
        run(&plain, at(2024, 2, 1, 0)).charges
    );
}

/// Ten seats, five removed for January 16 waiting for February 1, two bought on the 20th: twelve
/// seats less the ten billed, 1200 × 12/31 − 1000 × 12/31 = 78, then February bills 7.
/// Rules: R-108, R-079
#[test]
fn seats_bought_while_a_removal_waits_bill_on_top_of_the_seats_it_keeps() {
    let mut facts = seats(&[
        (d(2024, 1, 1), 10),
        (d(2024, 1, 16), 5),
        (d(2024, 1, 20), 7),
    ]);
    facts.seats[1].removed_at_renewal = 5;
    let evaluation = run(&facts, at(2024, 2, 1, 0));
    assert_eq!(
        of(&evaluation, 1)
            .iter()
            .map(|charge| (charge.amount, charge.quantity, charge.document))
            .collect::<Vec<_>>(),
        vec![
            (1_000, Some(Decimal::from(10)), recurring(d(2024, 1, 1))),
            (
                78,
                Some(Decimal::from(12)),
                DocumentSlot::Adjustment { on: d(2024, 1, 20) }
            ),
            (700, Some(Decimal::from(7)), recurring(d(2024, 2, 1))),
        ]
    );
}

/// Monthly from Jan 1, yearly from Jan 10 on the same grid: Feb 1 is inside the year, whose
/// renewal is Jan 1, 2025; a renewal is its own; an ending before any renewal leaves none.
/// Rules: R-106
#[test]
fn the_first_renewal_on_or_after_a_day_follows_the_cadence_in_force() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 12, 50_000));
    facts.plan_windows[0].to = Some(d(2024, 1, 10));
    facts
        .plan_windows
        .push(plan(100, d(2024, 1, 10), 12, d(2024, 1, 1)));
    let now = at(2024, 1, 10, 15);
    assert_eq!(
        renewal_on_or_after(&facts, d(2024, 2, 1), now),
        Some(d(2025, 1, 1))
    );
    assert_eq!(
        renewal_on_or_after(&facts, d(2025, 1, 1), now),
        Some(d(2025, 1, 1))
    );
    facts.ending = Some(Ending {
        at: midnight(d(2024, 6, 15)),
        kind: EndingKind::Cancel,
    });
    assert_eq!(renewal_on_or_after(&facts, d(2024, 2, 1), now), None);
}

fn with_usage(ending: Option<Ending>) -> Facts<TestFee> {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 3_100));
    facts.fees.push(metered(20, 2, d(2024, 1, 1), 1, 10));
    facts.ending = ending;
    facts
}

/// Cancelled on January 20 at 14:00: the fee bills its 19 days, the usage up to the instant, on
/// the invoice the cancellation opens; nothing is left to wake for.
/// Rules: R-016, R-077
#[test]
fn an_ending_mid_period_cuts_fees_at_its_date_and_usage_at_its_instant() {
    let ending = Ending {
        at: at(2024, 1, 20, 14),
        kind: EndingKind::Cancel,
    };
    let facts = with_usage(Some(ending));
    let now = at(2024, 1, 25, 0);
    let window = read(at(2024, 1, 1, 0), at(2024, 1, 20, 14));

    let unread = run(&facts, now);
    assert!(of(&unread, 2).is_empty());
    assert_eq!(
        unread.usage_needed,
        vec![UsageNeed {
            fee: lineage(2),
            read: window,
            document: recurring(d(2024, 1, 20)),
        }]
    );

    let usage = reads(&[(window, 7)]);
    let evaluation = evaluate(&facts, now, Some(&usage), &TestPricer).unwrap();
    assert!(evaluation.usage_needed.is_empty());
    let fee = of(&evaluation, 1);
    assert_eq!(fee.len(), 1);
    assert_eq!(fee[0].window, (d(2024, 1, 1), d(2024, 1, 20)));
    assert_eq!(fee[0].amount, 1_900);
    let metered = of(&evaluation, 2);
    assert_eq!(metered.len(), 1);
    assert_eq!(
        (metered[0].key.from, metered[0].key.until),
        (d(2024, 1, 1), d(2024, 1, 20))
    );
    assert_eq!(metered[0].amount, 70);
    assert_eq!(metered[0].due_at, ending.at);
    assert_eq!(metered[0].document, recurring(d(2024, 1, 20)));
    assert!(metered[0].trace.iter().any(|f| f.kind == FactKind::Ending));
    assert_eq!(evaluation.state.status, Status::Cancelled);
    assert_eq!(
        evaluation.state.current_period,
        Some((d(2024, 1, 20), None))
    );
    assert_eq!(evaluation.next_wake, None);

    let mut lines = issue(&run(&with_usage(None), at(2024, 1, 10, 0)).expected());
    let fee_key: Vec<Expected> = evaluation
        .expected()
        .into_iter()
        .filter(|e| e.key.fee == lineage(1))
        .collect();
    let adjustments = reconcile(&fee_key, &lines, TakeBack::Unused(d(2024, 1, 20)));
    assert_eq!(
        adjustments,
        vec![Adjustment::Credit {
            line: "l0".to_owned(),
            amount: 1_200
        }]
    );
    apply(&mut lines, &adjustments);
    assert!(reconcile(&fee_key, &lines, TakeBack::Unused(d(2024, 1, 20))).is_empty());
}

/// Cancelled at 10:00 on February 1, a renewal: January's usage on that renewal's invoice, the
/// morning's on a final invoice of its own.
/// Rules: R-016, R-011
#[test]
fn an_ending_on_a_renewal_bills_the_days_usage_on_a_final_invoice() {
    let ending = Ending {
        at: at(2024, 2, 1, 10),
        kind: EndingKind::Cancel,
    };
    let facts = with_usage(Some(ending));
    let january = read(at(2024, 1, 1, 0), at(2024, 2, 1, 0));
    let morning = read(at(2024, 2, 1, 0), at(2024, 2, 1, 10));
    let usage = reads(&[(january, 7), (morning, 2)]);
    let evaluation = evaluate(&facts, at(2024, 2, 2, 0), Some(&usage), &TestPricer).unwrap();
    let fee = of(&evaluation, 1);
    assert_eq!(periods(&fee), vec![(d(2024, 1, 1), d(2024, 2, 1))]);
    let metered = of(&evaluation, 2);
    assert_eq!(metered.len(), 2);
    assert_eq!(
        (metered[0].amount, metered[0].document),
        (70, recurring(d(2024, 2, 1)))
    );
    assert_eq!(
        (metered[1].key.from, metered[1].key.until),
        (d(2024, 2, 1), d(2024, 2, 1))
    );
    assert_eq!(
        (metered[1].amount, metered[1].document),
        (20, DocumentSlot::Final)
    );
    let lines = issue(&evaluation.expected());
    assert!(reconcile(&evaluation.expected(), &lines, TakeBack::LatestFirst).is_empty());
}

/// Started and cancelled on January 1 with nothing billed in advance: no invoice opened that
/// day, so the day's usage opens it rather than a final invoice.
/// Rules: R-016
#[test]
fn an_ending_the_day_billing_started_bills_usage_alone_on_its_opening_invoice() {
    let ending = Ending {
        at: at(2024, 1, 1, 10),
        kind: EndingKind::Cancel,
    };
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(metered(20, 2, d(2024, 1, 1), 1, 10));
    facts.ending = Some(ending);
    let morning = read(at(2024, 1, 1, 0), at(2024, 1, 1, 10));
    let usage = reads(&[(morning, 3)]);
    let evaluation = evaluate(&facts, at(2024, 1, 2, 0), Some(&usage), &TestPricer).unwrap();
    let metered = of(&evaluation, 2);
    assert_eq!(periods(&metered), vec![(d(2024, 1, 1), d(2024, 1, 1))]);
    assert_eq!(
        (metered[0].amount, metered[0].document),
        (30, recurring(d(2024, 1, 1)))
    );

    // An advance fee opened the day's invoice: the usage goes on a final one.
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 3_100));
    let evaluation = evaluate(&facts, at(2024, 1, 2, 0), Some(&usage), &TestPricer).unwrap();
    assert_eq!(of(&evaluation, 2)[0].document, DocumentSlot::Final);
}

/// A metered fee repriced from 0.10 to 0.20 a unit on Jan 20, the day a cancellation ends the
/// subscription at 14:00: the old row reads its days to that midnight, 7 units for 70, and the
/// new row the morning to the instant, on a line of that day alone, 3 units for 60. Removed
/// that day instead, the fee bills nothing of the morning.
/// Rules: R-016, R-075
#[test]
fn the_ending_days_usage_is_priced_by_the_row_in_force_that_day() {
    let ending = Ending {
        at: at(2024, 1, 20, 14),
        kind: EndingKind::Cancel,
    };
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.ending = Some(ending);
    let mut old = metered(20, 2, d(2024, 1, 1), 1, 10);
    old.to = Some(d(2024, 1, 20));
    facts.fees.push(old);
    let mut removed = facts.clone();
    facts.fees.push(metered(21, 2, d(2024, 1, 20), 1, 20));
    let days = read(at(2024, 1, 1, 0), at(2024, 1, 20, 0));
    let morning = read(at(2024, 1, 20, 0), ending.at);
    let usage = reads(&[(days, 7), (morning, 3)]);
    let charges = |facts: &Facts<TestFee>| -> Vec<_> {
        evaluate(facts, at(2024, 1, 25, 0), Some(&usage), &TestPricer)
            .unwrap()
            .charges
            .iter()
            .map(|c| ((c.key.from, c.key.until), c.usage, c.amount, c.document))
            .collect()
    };
    assert_eq!(
        charges(&facts),
        vec![
            (
                (d(2024, 1, 1), d(2024, 1, 20)),
                Some(days.window),
                70,
                recurring(d(2024, 1, 20))
            ),
            (
                (d(2024, 1, 20), d(2024, 1, 20)),
                Some(morning.window),
                60,
                recurring(d(2024, 1, 20))
            ),
        ]
    );
    removed.ending = Some(ending);
    assert_eq!(
        charges(&removed),
        vec![(
            (d(2024, 1, 1), d(2024, 1, 20)),
            Some(days.window),
            70,
            recurring(d(2024, 1, 20))
        )]
    );
}

/// A metered fee removed on January 15: its usage to then rides February's renewal, as an
/// arrears fee's would.
/// Rules: R-011
#[test]
fn arrears_ended_with_their_fee_ride_the_next_renewal() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    let mut removed = metered(20, 2, d(2024, 1, 1), 1, 10);
    removed.to = Some(d(2024, 1, 15));
    facts.fees.push(removed);
    let mut arrears = fixed(30, 3, d(2024, 1, 1), 1, 3_100);
    arrears.billing = Billing::Arrears;
    arrears.to = Some(d(2024, 1, 15));
    facts.fees.push(arrears);
    let half = read(at(2024, 1, 1, 0), at(2024, 1, 15, 0));
    let usage = reads(&[(half, 4)]);
    let evaluation = evaluate(&facts, at(2024, 2, 2, 0), Some(&usage), &TestPricer).unwrap();
    let metered = of(&evaluation, 2);
    assert_eq!(periods(&metered), vec![(d(2024, 1, 1), d(2024, 1, 15))]);
    assert_eq!(metered[0].document, recurring(d(2024, 2, 1)));
    let fee = of(&evaluation, 3);
    assert_eq!(fee[0].window, (d(2024, 1, 1), d(2024, 1, 15)));
    assert_eq!(fee[0].document, recurring(d(2024, 2, 1)));
}

/// A one-time price bills once, on its day, keyed on that day as its line is; one added later
/// bills on that day's adjustment invoice.
/// Rules: R-075
#[test]
fn a_one_time_fee_bills_on_its_day() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    let mut setup = fixed(10, 1, d(2024, 1, 1), 1, 5_000);
    setup.billing = Billing::OneTime;
    facts.fees.push(setup);
    let mut later = fixed(20, 2, d(2024, 1, 10), 1, 2_000);
    later.billing = Billing::OneTime;
    facts.fees.push(later);
    let evaluation = run(&facts, at(2024, 3, 1, 0));
    let setup = of(&evaluation, 1);
    assert_eq!(periods(&setup), vec![(d(2024, 1, 1), d(2024, 1, 1))]);
    assert_eq!(
        (setup[0].amount, setup[0].document),
        (5_000, recurring(d(2024, 1, 1)))
    );
    let later = of(&evaluation, 2);
    assert_eq!(periods(&later), vec![(d(2024, 1, 10), d(2024, 1, 10))]);
    assert_eq!(
        later[0].document,
        DocumentSlot::Adjustment { on: d(2024, 1, 10) }
    );
}

/// Rules: R-084
#[test]
fn a_yearly_fee_on_a_monthly_plan_bills_its_own_grid() {
    let mut facts = subscription(d(2024, 1, 10), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 10), 12, 12_000));
    facts.fees.push(fixed(20, 2, d(2024, 1, 10), 1, 3_100));
    let evaluation = run(&facts, at(2024, 2, 5, 0));
    let yearly = of(&evaluation, 1);
    assert_eq!(periods(&yearly), vec![(d(2024, 1, 10), d(2025, 1, 1))]);
    assert_eq!(yearly[0].amount, 11_705);
    assert_eq!(yearly[0].document, recurring(d(2024, 1, 10)));
    let monthly = of(&evaluation, 2);
    assert_eq!(
        periods(&monthly),
        vec![
            (d(2024, 1, 10), d(2024, 2, 1)),
            (d(2024, 2, 1), d(2024, 3, 1))
        ]
    );
    assert_eq!(monthly[0].amount, 2_200);
    assert_eq!(monthly[1].document, recurring(d(2024, 2, 1)));
}

/// A monthly fee on a quarterly plan: what falls due between two renewals rides the next one.
/// Rules: R-084
#[test]
fn a_fee_due_between_renewals_rides_the_next_renewal() {
    let mut facts = subscription(d(2024, 1, 1), 1, 3);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 1_000));
    facts.fees.push(metered(20, 2, d(2024, 1, 1), 1, 10));
    let usage = reads(&[
        (read(at(2024, 1, 1, 0), at(2024, 2, 1, 0)), 1),
        (read(at(2024, 2, 1, 0), at(2024, 3, 1, 0)), 2),
    ]);
    let evaluation = evaluate(&facts, at(2024, 3, 15, 0), Some(&usage), &TestPricer).unwrap();
    let documents = |fee| -> Vec<DocumentSlot> {
        of(&evaluation, fee)
            .iter()
            .map(|charge| charge.document)
            .collect()
    };
    assert_eq!(
        documents(1),
        vec![
            recurring(d(2024, 1, 1)),
            recurring(d(2024, 4, 1)),
            recurring(d(2024, 4, 1))
        ]
    );
    assert_eq!(
        documents(2),
        vec![recurring(d(2024, 4, 1)), recurring(d(2024, 4, 1))]
    );
    assert_eq!(
        evaluation.state.current_period,
        Some((d(2024, 1, 1), Some(d(2024, 4, 1))))
    );
    assert_eq!(
        evaluation.usage_needed,
        vec![UsageNeed {
            fee: lineage(2),
            read: read(at(2024, 3, 1, 0), at(2024, 4, 1, 0)),
            document: recurring(d(2024, 4, 1)),
        }]
    );
}

#[test]
fn a_fee_added_mid_period_bills_what_is_left_on_an_adjustment_invoice() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 10), 1, 3_100));
    let evaluation = run(&facts, at(2024, 1, 12, 0));
    let charges = of(&evaluation, 1);
    assert_eq!(charges[0].window, (d(2024, 1, 10), d(2024, 2, 1)));
    assert_eq!(charges[0].amount, 2_200);
    assert_eq!(
        charges[0].document,
        DocumentSlot::Adjustment { on: d(2024, 1, 10) }
    );
}

/// A capacity billed whole changed on January 17 bills its difference for what is left of the
/// month, as any change does; billed whole again at the renewal.
/// Rules: R15, R-079
#[test]
fn a_change_mid_period_prorates_a_fee_billed_whole() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    let mut small = fixed(10, 1, d(2024, 1, 1), 1, 3_100);
    small.fee.whole = true;
    small.to = Some(d(2024, 1, 17));
    let mut large = fixed(11, 1, d(2024, 1, 17), 1, 6_200);
    large.fee.whole = true;
    facts.fees.extend([small, large]);
    let evaluation = run(&facts, at(2024, 2, 10, 0));
    let charges = of(&evaluation, 1);
    let amounts: Vec<(i64, DocumentSlot)> =
        charges.iter().map(|c| (c.amount, c.document)).collect();
    assert_eq!(
        amounts,
        vec![
            (3_100, recurring(d(2024, 1, 1))),
            (1_500, DocumentSlot::Adjustment { on: d(2024, 1, 17) }),
            (6_200, recurring(d(2024, 2, 1))),
        ]
    );
}

/// A capacity billed whole, ended on January 17 by a change of plan, owes what it used: 3100 less
/// what is left of January, 3100 × 15/31 = 1500, so 1600. A cancellation or a fixed term ending
/// that day cuts it the same.
/// Rules: R-002, R-077, R-084
#[test]
fn a_fee_billed_whole_cut_short_owes_what_it_used() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    let mut capacity = fixed(10, 1, d(2024, 1, 1), 1, 3_100);
    capacity.fee.whole = true;
    capacity.to = Some(d(2024, 1, 17));
    facts.fees.push(capacity);
    let amounts = |facts: &Facts<TestFee>| -> Vec<i64> {
        of(&run(facts, at(2024, 1, 20, 0)), 1)
            .iter()
            .map(|charge| charge.amount)
            .collect()
    };
    assert_eq!(amounts(&facts), vec![1_600]);

    let mut cancelled = facts.clone();
    cancelled.fees[0].to = None;
    cancelled.ending = Some(Ending {
        at: midnight(d(2024, 1, 17)),
        kind: EndingKind::Cancel,
    });
    assert_eq!(amounts(&cancelled), vec![1_600]);

    let mut completed = facts.clone();
    completed.fees[0].to = None;
    completed.end_date = Some(d(2024, 1, 17));
    assert_eq!(amounts(&completed), vec![1_600]);
}

/// A 31.00 fee in arrears ended on January 17 owes what it used, 3100 less 3100 × 15/31 = 1500,
/// so 1600, billed whole or prorated, when its row, a cancellation or a fixed term ends it.
/// Rules: R-002, R-016, R-084
#[test]
fn arrears_cut_short_owe_what_they_used() {
    for whole in [false, true] {
        let mut facts = subscription(d(2024, 1, 1), 1, 1);
        let mut arrears = fixed(10, 1, d(2024, 1, 1), 1, 3_100);
        arrears.billing = Billing::Arrears;
        arrears.fee.whole = whole;
        arrears.to = Some(d(2024, 1, 17));
        facts.fees.push(arrears);
        let amounts = |facts: &Facts<TestFee>| -> Vec<i64> {
            of(&run(facts, at(2024, 2, 2, 0)), 1)
                .iter()
                .map(|charge| charge.amount)
                .collect()
        };
        assert_eq!(amounts(&facts), vec![1_600], "removed, whole {whole}");

        let mut cancelled = facts.clone();
        cancelled.fees[0].to = None;
        cancelled.ending = Some(Ending {
            at: midnight(d(2024, 1, 17)),
            kind: EndingKind::Cancel,
        });
        assert_eq!(amounts(&cancelled), vec![1_600], "cancelled, whole {whole}");

        let mut completed = facts.clone();
        completed.fees[0].to = None;
        completed.end_date = Some(d(2024, 1, 17));
        assert_eq!(amounts(&completed), vec![1_600], "completed, whole {whole}");
    }
}

/// The usage of `facts`' fee 1 evaluated on Feb 2 with `entries` read, by charge period.
fn usage_charges(
    facts: &Facts<TestFee>,
    entries: &[(UsageRead, i64)],
) -> Vec<((NaiveDate, NaiveDate), i64)> {
    let usage = reads(entries);
    let evaluation = evaluate(facts, at(2024, 2, 2, 0), Some(&usage), &TestPricer).unwrap();
    of(&evaluation, 1)
        .iter()
        .filter(|charge| charge.usage.is_some())
        .map(|charge| ((charge.key.from, charge.key.until), charge.amount))
        .collect()
}

/// A lineage of two rows split on Jan 16, 900 units read on each part and 1800 over January.
fn split_usage(fee: TestFee) -> (Facts<TestFee>, Vec<(UsageRead, i64)>) {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    let row = |id, from| FeeFact {
        fee: fee.clone(),
        ..metered(id, 1, from, 1, fee.unit)
    };
    facts.fees.push(FeeFact {
        to: Some(d(2024, 1, 16)),
        ..row(10, d(2024, 1, 1))
    });
    facts.fees.push(row(11, d(2024, 1, 16)));
    let entries = vec![
        (read(at(2024, 1, 1, 0), at(2024, 1, 16, 0)), 900),
        (read(at(2024, 1, 16, 0), at(2024, 2, 1, 0)), 900),
        (read(at(2024, 1, 1, 0), at(2024, 2, 1, 0)), 1_800),
    ];
    (facts, entries)
}

/// A capacity of 1000 units at 0.10 over, its row replaced on Jan 16 with 900 units used on each
/// side: the allowance is counted once over January, (1800 − 1000) × 10 = 8000, never twice.
/// Rules: R-097
#[test]
fn a_capacity_split_mid_period_counts_its_allowance_once() {
    let (facts, entries) = split_usage(TestFee {
        metric: Some(id(METRIC)),
        unit: 10,
        included: 1_000,
        per_period: true,
        ..TestFee::default()
    });
    assert_eq!(
        usage_charges(&facts, &entries),
        vec![((d(2024, 1, 1), d(2024, 2, 1)), 8_000)]
    );
}

/// A capacity of 1000 units at 0.10 over, moved on Jan 16 to a level of 2000 units, 900 units used
/// on each side: each part counts its usage against its own level's share of the days. January 1
/// to 16 holds 15/31 of 1000, 483.87 units, so 416.13 over bill 4161; January 16 to February 1
/// holds 16/31 of 2000, 1032.26 units, and 900 bill nothing. Read whole at the new level, the
/// 1800 units would bill nothing.
/// Rules: R-135, R-002
#[test]
fn a_capacity_level_changed_mid_period_counts_each_part_against_its_level() {
    let (mut facts, entries) = split_usage(TestFee {
        metric: Some(id(METRIC)),
        unit: 10,
        included: 1_000,
        per_period: true,
        ..TestFee::default()
    });
    facts.fees[1].fee.included = 2_000;
    assert_eq!(
        usage_charges(&facts, &entries),
        vec![
            ((d(2024, 1, 1), d(2024, 1, 16)), 4_161),
            ((d(2024, 1, 16), d(2024, 2, 1)), 0),
        ]
    );
}

/// Graduated tiers of 0.10 up to 1000 units and 0.05 past them, the row replaced on Jan 16 with
/// 900 units used on each side: the tiers run over January, 1000 × 10 + 800 × 5 = 14000.
/// Rules: R-097
#[test]
fn graduated_tiers_split_mid_period_run_over_the_whole_period() {
    let (facts, entries) = split_usage(TestFee {
        metric: Some(id(METRIC)),
        unit: 10,
        tier: Some((1_000, 5)),
        per_period: true,
        ..TestFee::default()
    });
    assert_eq!(
        usage_charges(&facts, &entries),
        vec![((d(2024, 1, 1), d(2024, 2, 1)), 14_000)]
    );
}

/// Usage priced by the unit keeps each row's own price: 900 units at 0.10, then 900 at 0.20.
/// Rules: R-097
#[test]
fn usage_by_the_unit_split_mid_period_bills_each_part_at_its_price() {
    let (mut facts, entries) = split_usage(TestFee {
        metric: Some(id(METRIC)),
        unit: 10,
        ..TestFee::default()
    });
    facts.fees[1].fee.unit = 20;
    assert_eq!(
        usage_charges(&facts, &entries),
        vec![
            ((d(2024, 1, 1), d(2024, 1, 16)), 9_000),
            ((d(2024, 1, 16), d(2024, 2, 1)), 18_000),
        ]
    );
}

/// A capacity of 310 units at 0.10 over, ended on Jan 17 by a fixed term or a cancellation:
/// its fee is prorated to 16 of 31 days, and its allowance with it, 160 units, so 200 used owe
/// 40 × 10 = 400; run to the period's end, the same 200 owe nothing.
/// Rules: R-002, R-084
#[test]
fn a_capacity_cut_short_prorates_its_allowance_with_its_fee() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(FeeFact {
        billing: Billing::Advance,
        fee: TestFee {
            metric: Some(id(METRIC)),
            unit: 10,
            included: 310,
            per_period: true,
            ..TestFee::default()
        },
        ..metered(10, 1, d(2024, 1, 1), 1, 10)
    });
    let to_17 = (read(at(2024, 1, 1, 0), at(2024, 1, 17, 0)), 200);
    let whole = (read(at(2024, 1, 1, 0), at(2024, 2, 1, 0)), 200);
    assert_eq!(
        usage_charges(&facts, &[whole]),
        vec![((d(2024, 1, 1), d(2024, 2, 1)), 0)]
    );

    let mut completed = facts.clone();
    completed.end_date = Some(d(2024, 1, 17));
    assert_eq!(
        usage_charges(&completed, &[to_17]),
        vec![((d(2024, 1, 1), d(2024, 1, 17)), 400)]
    );

    let mut cancelled = facts.clone();
    cancelled.ending = Some(Ending {
        at: midnight(d(2024, 1, 17)),
        kind: EndingKind::Cancel,
    });
    assert_eq!(
        usage_charges(&cancelled, &[to_17]),
        vec![((d(2024, 1, 1), d(2024, 1, 17)), 400)]
    );
}

/// A capacity of 310 units at 0.10 over, cancelled at an instant: on Jan 17 at 10:00 its fee
/// holds 16 of 31 days, 160 units, so 200 used owe 400; on Feb 1 at 10:00 the ending day's usage
/// is in a period its fee holds no day of, so its 20 units owe 200.
/// Rules: R-002, R-016
#[test]
fn a_cancellations_last_usage_counts_the_allowance_its_fee_holds() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(FeeFact {
        billing: Billing::Advance,
        fee: TestFee {
            metric: Some(id(METRIC)),
            unit: 10,
            included: 310,
            per_period: true,
            ..TestFee::default()
        },
        ..metered(10, 1, d(2024, 1, 1), 1, 10)
    });
    let cancelled = |at: DateTime<Utc>| Facts {
        ending: Some(Ending {
            at,
            kind: EndingKind::Cancel,
        }),
        ..facts.clone()
    };
    assert_eq!(
        usage_charges(
            &cancelled(at(2024, 1, 17, 10)),
            &[(read(at(2024, 1, 1, 0), at(2024, 1, 17, 10)), 200)]
        ),
        vec![((d(2024, 1, 1), d(2024, 1, 17)), 400)]
    );
    assert_eq!(
        usage_charges(
            &cancelled(at(2024, 2, 1, 10)),
            &[
                (read(at(2024, 1, 1, 0), at(2024, 2, 1, 0)), 200),
                (read(at(2024, 2, 1, 0), at(2024, 2, 1, 10)), 20),
            ]
        ),
        vec![
            ((d(2024, 1, 1), d(2024, 2, 1)), 0),
            ((d(2024, 2, 1), d(2024, 2, 1)), 200)
        ]
    );
}

/// Started January 10 on a grid anchored on the 1st, a 31.00 fee in arrears bills its first
/// period's 22 of January's 31 days on February 1, 3100 × 22/31 = 2200, as the advance stub is
/// prorated; then whole periods.
/// Rules: R-002, R-084
#[test]
fn a_first_partial_periods_fixed_arrears_are_prorated() {
    let mut facts = subscription(d(2024, 1, 10), 1, 1);
    let mut arrears = fixed(10, 1, d(2024, 1, 10), 1, 3_100);
    arrears.billing = Billing::Arrears;
    facts.fees.push(arrears);
    let evaluation = run(&facts, at(2024, 3, 1, 0));
    let charges = of(&evaluation, 1);
    assert_eq!(
        charges
            .iter()
            .map(|charge| (charge.key.from, charge.amount, charge.document))
            .collect::<Vec<_>>(),
        vec![
            (d(2024, 1, 10), 2_200, recurring(d(2024, 2, 1))),
            (d(2024, 2, 1), 3_100, recurring(d(2024, 3, 1))),
        ]
    );
}

/// Started January 10 on a grid anchored on the 1st, a capacity of 31.00 for 310 units bills its
/// first period's 22 of 31 days, 2200, and includes 220 units: 240 used owe 20 × 10 = 200. In
/// arrears its fee is the same 2200; then whole periods, 3100 with 310 units.
/// Rules: R-002, R-084
#[test]
fn a_first_partial_period_prorates_a_capacitys_fee_and_allowance() {
    for billing in [Billing::Advance, Billing::Arrears] {
        let mut facts = subscription(d(2024, 1, 10), 1, 1);
        facts.fees.push(FeeFact {
            billing: Billing::Advance,
            fee: TestFee {
                metric: Some(id(METRIC)),
                unit: 10,
                included: 310,
                per_period: true,
                ..TestFee::default()
            },
            ..metered(10, 1, d(2024, 1, 10), 1, 10)
        });
        let mut fee = fixed(20, 2, d(2024, 1, 10), 1, 3_100);
        fee.fee.whole = true;
        fee.billing = billing;
        facts.fees.push(fee);
        let usage = reads(&[
            (read(at(2024, 1, 10, 0), at(2024, 2, 1, 0)), 240),
            (read(at(2024, 2, 1, 0), at(2024, 3, 1, 0)), 320),
        ]);
        let evaluation = evaluate(&facts, at(2024, 3, 1, 0), Some(&usage), &TestPricer).unwrap();
        let amounts = |fee: u128| -> Vec<(NaiveDate, i64)> {
            of(&evaluation, fee)
                .iter()
                .map(|charge| (charge.key.from, charge.amount))
                .collect()
        };
        assert_eq!(
            amounts(2)[..2],
            [(d(2024, 1, 10), 2_200), (d(2024, 2, 1), 3_100)],
            "{billing:?}"
        );
        assert_eq!(
            amounts(1),
            vec![(d(2024, 1, 10), 200), (d(2024, 2, 1), 100)],
            "{billing:?}"
        );
    }
}

/// A pack with its own duration added on January 17 is bought whole, keyed on its day as its
/// line is; from February it bills its period.
/// Rules: R-075
#[test]
fn a_pack_bought_whole_at_a_change_is_keyed_on_its_day() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    let mut pack = fixed(10, 1, d(2024, 1, 17), 1, 5_000);
    pack.fee.whole = true;
    pack.fee.grant = Some(5_000);
    facts.fees.push(pack);
    let evaluation = run(&facts, at(2024, 2, 10, 0));
    let charges = of(&evaluation, 1);
    assert_eq!(
        periods(&charges),
        vec![
            (d(2024, 1, 17), d(2024, 1, 17)),
            (d(2024, 2, 1), d(2024, 3, 1))
        ]
    );
    assert_eq!(charges[0].amount, 5_000);
    assert_eq!(
        charges[0].credit_grant.map(|grant| grant.amount),
        Some(5_000)
    );
}

/// A pack bought whole on Jan 17, recurring or once: keyed on its day and billing that day alone,
/// as the change's line shows it, so that line bills the charge and a second change that day
/// finds it billed.
/// Rules: R-075
#[test]
fn a_pack_bought_whole_at_a_change_bills_its_day_alone() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    for (row, billing) in [(10, Billing::Advance), (20, Billing::OneTime)] {
        let mut pack = fixed(row, row, d(2024, 1, 17), 1, 5_000);
        pack.fee.whole = true;
        pack.fee.grant = Some(5_000);
        pack.billing = billing;
        facts.fees.push(pack);
    }
    let evaluation = run(&facts, at(2024, 1, 20, 0));
    for fee in [10, 20] {
        let charges = of(&evaluation, fee);
        let day = (d(2024, 1, 17), d(2024, 1, 17));
        assert_eq!(
            charges
                .iter()
                .map(|c| ((c.key.from, c.key.until), c.window, c.amount))
                .collect::<Vec<_>>(),
            vec![(day, day, 5_000)],
            "{fee}"
        );
        let as_shown = line("pack".into(), &charges[0].key, 5_000);
        assert!(as_shown.bills(&charges[0].key));
    }
}

/// A 50.00 pack repriced to 80.00 on January 17 is never netted: bought whole, the new pack bills
/// 8000 on its day, granting 8000; tied to its period, its stub, 8000 × 15/31 = 3871. The old
/// pack is given back by its credit's settlement alone: no charge of the change takes it back,
/// nor one of a pack ended that day.
/// Rules: R-103, OQ1, R-075
#[test]
fn a_pack_is_never_netted() {
    let charges = |whole: bool, repriced: bool| -> Vec<_> {
        let mut facts = subscription(d(2024, 1, 1), 1, 1);
        let mut old = fixed(10, 1, d(2024, 1, 1), 1, 5_000);
        old.fee.whole = whole;
        old.fee.grant = Some(5_000);
        old.to = Some(d(2024, 1, 17));
        facts.fees.push(old);
        if repriced {
            let mut new = fixed(11, 1, d(2024, 1, 17), 1, 8_000);
            new.fee.whole = whole;
            new.fee.grant = Some(8_000);
            facts.fees.push(new);
        }
        of(&run(&facts, at(2024, 1, 20, 0)), 1)
            .iter()
            .map(|c| {
                (
                    (c.key.from, c.key.until),
                    c.amount,
                    c.credit_grant.map(|g| g.amount),
                )
            })
            .collect()
    };
    let january = (d(2024, 1, 1), d(2024, 2, 1));
    let day = (d(2024, 1, 17), d(2024, 1, 17));
    assert_eq!(
        charges(true, true),
        vec![(january, 5_000, Some(5_000)), (day, 8_000, Some(8_000))]
    );
    assert_eq!(charges(true, false), vec![(january, 5_000, Some(5_000))]);
    assert_eq!(
        charges(false, true),
        vec![(january, 5_000, Some(5_000)), (january, 3_871, Some(3_871))]
    );
}

/// The new plan's one-time fee bills nothing on a change: a plan's setup is billed with the
/// subscription's start.
/// Rules: R-093
#[test]
fn a_plan_change_bills_no_one_time_fee() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.plan_windows[0].to = Some(d(2024, 2, 1));
    facts
        .plan_windows
        .push(plan(101, d(2024, 2, 1), 1, d(2024, 1, 1)));
    let mut setup = fixed(10, 1, d(2024, 2, 1), 1, 5_000);
    setup.billing = Billing::OneTime;
    facts.fees.push(setup);
    assert!(of(&run(&facts, at(2024, 3, 1, 0)), 1).is_empty());
}

/// An amendment on Mar 15 moving a yearly plan onto the monthly grid, the version kept, and adding
/// a 50.00 one-time add-on: the add-on bills 5000 on that day's adjustment invoice. A plan's
/// one-time fee is left out only on a change of plan version.
/// Rules: R-093, R-075
#[test]
fn a_one_time_add_on_bought_with_a_change_of_cadence_bills() {
    let mut facts = subscription(d(2024, 1, 1), 1, 12);
    facts
        .plan_windows
        .push(plan(100, d(2024, 3, 15), 1, d(2024, 1, 1)));
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 12, 12_000));
    let mut add_on = fixed(20, 2, d(2024, 3, 15), 1, 5_000);
    add_on.lineage = Fee::AddOn(id(2));
    add_on.billing = Billing::OneTime;
    add_on.plan_component = false;
    add_on.fee.whole = true;
    facts.fees.push(add_on);
    let evaluation = run(&facts, at(2024, 3, 20, 0));
    let charges: Vec<_> = evaluation
        .charges
        .iter()
        .filter(|c| c.key.fee == Fee::AddOn(id(2)))
        .map(|c| ((c.key.from, c.key.until), c.amount, c.document))
        .collect();
    assert_eq!(
        charges,
        vec![(
            (d(2024, 3, 15), d(2024, 3, 15)),
            5_000,
            DocumentSlot::Adjustment { on: d(2024, 3, 15) }
        )]
    );
}

/// A 99.00 one-time fee added on Jan 1 in a free trial to Jan 15, the subscription cancelled at
/// the trial's end: the fee lived before the ending and bills 9900 on Jan 15, where no window
/// was left to bill it. Cancelled inside the trial on Jan 10, it bills on that day; a change
/// ending the trial and the fee's row on Jan 4 bills it on the change's opening invoice.
/// Rules: R-093, R-016, R-011
#[test]
fn a_one_time_fee_added_in_a_free_trial_bills_when_an_ending_closes_the_trial() {
    let mut facts = subscription(d(2025, 1, 1), 1, 1);
    facts.trial = Some(Trial {
        from: d(2025, 1, 1),
        until: d(2025, 1, 15),
        free: true,
    });
    facts.fees.push(fixed(10, 1, d(2025, 1, 1), 1, 2_900));
    let mut setup = fixed(20, 2, d(2025, 1, 1), 1, 9_900);
    setup.billing = Billing::OneTime;
    setup.plan_component = false;
    setup.fee.whole = true;
    facts.fees.push(setup);
    let billed = |facts: &Facts<TestFee>, now| {
        run(facts, now)
            .charges
            .iter()
            .map(|c| {
                (
                    c.key.fee.clone(),
                    (c.key.from, c.key.until),
                    c.amount,
                    c.document,
                )
            })
            .collect::<Vec<_>>()
    };

    facts.ending = Some(Ending {
        at: at(2025, 1, 15, 0),
        kind: EndingKind::Cancel,
    });
    assert_eq!(
        billed(&facts, at(2025, 1, 16, 0)),
        vec![(
            lineage(2),
            (d(2025, 1, 15), d(2025, 1, 15)),
            9_900,
            recurring(d(2025, 1, 15))
        )]
    );

    facts.ending = Some(Ending {
        at: at(2025, 1, 10, 0),
        kind: EndingKind::Cancel,
    });
    assert_eq!(
        billed(&facts, at(2025, 1, 11, 0)),
        vec![(
            lineage(2),
            (d(2025, 1, 10), d(2025, 1, 10)),
            9_900,
            recurring(d(2025, 1, 10))
        )]
    );

    // Added after the ending's day, it never lived.
    facts.fees[1].from = d(2025, 1, 10);
    assert_eq!(billed(&facts, at(2025, 1, 11, 0)), vec![]);

    facts.fees[1].from = d(2025, 1, 1);
    facts.fees[1].to = Some(d(2025, 1, 4));
    facts.ending = None;
    facts.trial = facts.trial.map(|trial| Trial {
        until: d(2025, 1, 4),
        ..trial
    });
    facts.fees[0].to = Some(d(2025, 1, 4));
    facts.fees.push(fixed(11, 3, d(2025, 1, 4), 1, 4_999));
    let charges = billed(&facts, at(2025, 1, 5, 0));
    assert!(charges.contains(&(
        lineage(2),
        (d(2025, 1, 4), d(2025, 1, 4)),
        9_900,
        recurring(d(2025, 1, 4))
    )));
    // Removed inside the trial, before its end, it never bills.
    facts.fees[1].to = Some(d(2025, 1, 3));
    assert!(
        billed(&facts, at(2025, 1, 5, 0))
            .iter()
            .all(|charge| charge.0 != lineage(2))
    );
}

/// 49.99 a year in arrears from Jan 25, 2025, renewing monthly, cancelled on Feb 25 from that
/// day: the year's 31 days the ending cuts, 4999 × 31/365 = 425, go on a final invoice when Feb
/// 25's renewal was written before the cancellation was decided, else on that renewal.
/// Rules: R-016
#[test]
fn a_cancellation_after_its_day_invoice_was_written_bills_its_cut_fees_on_a_final_invoice() {
    let mut facts = subscription(d(2025, 1, 25), 25, 1);
    let mut yearly = fixed(10, 1, d(2025, 1, 25), 12, 4_999);
    yearly.billing = Billing::Arrears;
    facts.fees.push(yearly);
    facts.fees.push(fixed(20, 2, d(2025, 1, 25), 1, 9_900));
    facts.ending = Some(Ending {
        at: at(2025, 2, 25, 0),
        kind: EndingKind::Cancel,
    });
    facts.ending_recorded = Some(d(2025, 2, 25));
    let billed = |facts: &Facts<TestFee>| {
        of(&run(facts, at(2025, 2, 26, 0)), 1)
            .iter()
            .map(|c| ((c.key.from, c.key.until), c.amount, c.document))
            .collect::<Vec<_>>()
    };
    facts.ending_day_invoiced = true;
    assert_eq!(
        billed(&facts),
        vec![((d(2025, 1, 25), d(2026, 1, 25)), 425, DocumentSlot::Final)]
    );
    // Decided that day before the renewal was written, the renewal carries it.
    facts.ending_day_invoiced = false;
    assert_eq!(
        billed(&facts),
        vec![(
            (d(2025, 1, 25), d(2026, 1, 25)),
            425,
            recurring(d(2025, 2, 25))
        )]
    );
}

/// 500.00 a month in arrears from Jan 7, 2025, cancelled from Feb 7: requested on Feb 9, after
/// its day, January's 50000 goes on the final invoice issued then, unless the recurring invoice
/// of Feb 7 was issued already, which keeps it; requested before, on that recurring invoice.
/// Rules: R-016, R-011
#[test]
fn a_backdated_cancellation_bills_the_last_period_arrears_on_its_final_invoice() {
    let mut facts = subscription(d(2025, 1, 7), 7, 1);
    let mut monthly = fixed(10, 1, d(2025, 1, 7), 1, 50_000);
    monthly.billing = Billing::Arrears;
    facts.fees.push(monthly);
    facts.ending = Some(Ending {
        at: at(2025, 2, 7, 0),
        kind: EndingKind::Cancel,
    });
    let billed = |facts: &Facts<TestFee>| {
        of(&run(facts, at(2025, 2, 9, 9)), 1)
            .iter()
            .map(|c| ((c.key.from, c.key.until), c.amount, c.document))
            .collect::<Vec<_>>()
    };
    facts.ending_recorded = Some(d(2025, 2, 9));
    assert_eq!(
        billed(&facts),
        vec![((d(2025, 1, 7), d(2025, 2, 7)), 50_000, DocumentSlot::Final)]
    );
    facts.ending_day_invoiced = true;
    assert_eq!(
        billed(&facts),
        vec![(
            (d(2025, 1, 7), d(2025, 2, 7)),
            50_000,
            recurring(d(2025, 2, 7))
        )]
    );
    facts.ending_day_invoiced = false;
    facts.ending_recorded = Some(d(2025, 1, 20));
    assert_eq!(
        billed(&facts),
        vec![(
            (d(2025, 1, 7), d(2025, 2, 7)),
            50_000,
            recurring(d(2025, 2, 7))
        )]
    );
}

/// A row starting later bills over an earlier one of its lineage still open.
#[test]
fn the_later_fee_row_of_a_lineage_takes_precedence() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 1_000));
    facts.fees.push(fixed(11, 1, d(2024, 2, 1), 1, 2_000));
    let evaluation = run(&facts, at(2024, 2, 5, 0));
    let charges = of(&evaluation, 1);
    assert_eq!(
        charges.iter().map(|c| c.amount).collect::<Vec<_>>(),
        vec![1_000, 2_000]
    );
    let row = |n| FactRef {
        kind: FactKind::Fee,
        id: id(n),
    };
    assert!(charges[0].trace.contains(&row(10)));
    assert!(charges[1].trace.contains(&row(11)));
    assert!(!charges[1].trace.contains(&row(10)));
}

#[test]
fn a_fixed_term_ending_first_completes_the_subscription() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 1_000));
    facts.end_date = Some(d(2024, 3, 1));
    facts.ending = Some(Ending {
        at: at(2024, 4, 1, 0),
        kind: EndingKind::Cancel,
    });
    let evaluation = run(&facts, at(2024, 3, 5, 0));
    assert_eq!(evaluation.state.status, Status::Completed);
    assert_eq!(of(&evaluation, 1).len(), 2);
    assert_eq!(evaluation.next_wake, None);
}

/// A cancellation dated on the fixed term's end: the term ends the subscription, Completed.
#[test]
fn a_cancellation_on_the_fixed_terms_end_completes_the_subscription() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 1_000));
    facts.end_date = Some(d(2024, 3, 1));
    facts.ending = Some(Ending {
        at: at(2024, 3, 1, 0),
        kind: EndingKind::Cancel,
    });
    let evaluation = run(&facts, at(2024, 3, 5, 0));
    assert_eq!(evaluation.state.status, Status::Completed);
    assert_eq!(of(&evaluation, 1).len(), 2);
    assert_eq!(evaluation.next_wake, None);
}

/// The minimum emits its period's commitment when the period closes; its shortfall is
/// reconcile's, and the commitment is no money of its own.
/// Rules: R-022
#[test]
fn a_minimum_commits_its_period_when_the_period_closes() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 1_000));
    facts.minimums.push(MinimumFact {
        id: id(700),
        from: d(2024, 1, 1),
        to: None,
        amount: 5_000,
        scope: MinimumScope::AllComponents,
    });
    facts.coupons.push(CouponFact {
        id: id(800),
        from: d(2024, 1, 1),
        invoices: Some(3),
        until: None,
    });
    let evaluation = run(&facts, at(2024, 2, 15, 0));
    let commitments: Vec<&Charge> = evaluation
        .charges
        .iter()
        .filter(|charge| charge.is_commitment())
        .collect();
    assert_eq!(commitments.len(), 1);
    assert_eq!(commitments[0].key.fee, Fee::Minimum { product: None });
    assert_eq!(
        (commitments[0].key.from, commitments[0].key.until),
        (d(2024, 1, 1), d(2024, 2, 1))
    );
    assert_eq!(commitments[0].amount, 5_000);
    assert_eq!(commitments[0].document, recurring(d(2024, 2, 1)));
    assert!(
        evaluation
            .expected()
            .iter()
            .all(|e| e.key.kind == ChargeKind::Fee)
    );
    assert_eq!(
        evaluation.state.first_period,
        Some((d(2024, 1, 1), d(2024, 2, 1)))
    );
}

/// A yearly 366.00 fee in arrears removed on Mar 15, its year not settled by a watermark on
/// Jun 1: it owes 36600 × 74/366 = 7400, once, on the first renewal past the watermark, Jul 1;
/// Apr 1, the next renewal, was the per-operation code's. Its usage likewise.
/// Rules: R-011, R-094
#[test]
fn arrears_ended_before_the_watermark_ride_the_first_renewal_past_it() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    let mut arrears = fixed(10, 1, d(2024, 1, 1), 12, 36_600);
    arrears.billing = Billing::Arrears;
    arrears.to = Some(d(2024, 3, 15));
    facts.fees.push(arrears);
    let mut removed = metered(20, 2, d(2024, 1, 1), 12, 10);
    removed.to = Some(d(2024, 3, 15));
    facts.fees.push(removed);
    facts.reconcile_from = Some(d(2024, 6, 1));
    let used = read(at(2024, 1, 1, 0), at(2024, 3, 15, 0));
    let evaluation = evaluate(
        &facts,
        at(2024, 7, 2, 0),
        Some(&reads(&[(used, 4)])),
        &TestPricer,
    )
    .unwrap();
    let charges: Vec<_> = evaluation
        .charges
        .iter()
        .map(|c| (c.key.fee.clone(), c.window, c.amount, c.document))
        .collect();
    assert_eq!(
        charges,
        vec![
            (
                lineage(1),
                (d(2024, 1, 1), d(2024, 3, 15)),
                7_400,
                recurring(d(2024, 7, 1))
            ),
            (
                lineage(2),
                (d(2024, 1, 1), d(2024, 3, 15)),
                40,
                recurring(d(2024, 7, 1))
            ),
        ]
    );
}

#[test]
fn a_watermark_leaves_out_the_periods_it_settled() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 3_100));
    facts.fees.push(metered(20, 2, d(2024, 1, 1), 1, 10));
    facts.reconcile_from = Some(d(2024, 3, 15));
    let evaluation = run(&facts, at(2024, 4, 10, 0));
    assert_eq!(
        periods(&of(&evaluation, 1)),
        vec![
            (d(2024, 3, 1), d(2024, 4, 1)),
            (d(2024, 4, 1), d(2024, 5, 1))
        ]
    );
    assert_eq!(
        evaluation.usage_needed,
        vec![
            UsageNeed {
                fee: lineage(2),
                read: read(at(2024, 3, 1, 0), at(2024, 4, 1, 0)),
                document: recurring(d(2024, 4, 1)),
            },
            UsageNeed {
                fee: lineage(2),
                read: read(at(2024, 4, 1, 0), at(2024, 5, 1, 0)),
                document: recurring(d(2024, 5, 1)),
            },
        ]
    );
}

#[test]
fn an_open_usage_window_is_only_reported() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(metered(20, 2, d(2024, 1, 1), 1, 10));
    let window = read(at(2024, 1, 1, 0), at(2024, 2, 1, 0));
    let usage = reads(&[(window, 5)]);
    let evaluation = evaluate(&facts, at(2024, 1, 15, 0), Some(&usage), &TestPricer).unwrap();
    assert!(evaluation.charges.is_empty());
    assert_eq!(
        evaluation.usage_needed,
        vec![UsageNeed {
            fee: lineage(2),
            read: window,
            document: recurring(d(2024, 2, 1)),
        }]
    );
    assert_eq!(evaluation.next_wake, Some(at(2024, 2, 1, 0)));
}

/// The usage windows of `facts` begun by `now`: row, days, read, document.
fn windows_of(
    facts: &Facts<TestFee>,
    now: DateTime<Utc>,
) -> Vec<(u128, (NaiveDate, NaiveDate), UsageRead, DocumentSlot)> {
    usage_windows(facts, now, &TestPricer)
        .unwrap()
        .iter()
        .map(|window| {
            (
                window.row.id.as_u128(),
                window.days,
                UsageRead {
                    metric: id(METRIC),
                    window: window.read,
                },
                window.document,
            )
        })
        .collect()
}

/// Metered from January 1 on a monthly grid, read on March 15: January and February on the
/// renewals ending them, March open on April's; each as `usage_needed` lists it.
/// Rules: R-084
#[test]
fn usage_windows_follow_the_fees_grid_open_window_included() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(metered(20, 2, d(2024, 1, 1), 1, 10));
    let now = at(2024, 3, 15, 0);
    let windows = windows_of(&facts, now);
    assert_eq!(
        windows,
        vec![
            (
                20,
                (d(2024, 1, 1), d(2024, 2, 1)),
                read(at(2024, 1, 1, 0), at(2024, 2, 1, 0)),
                recurring(d(2024, 2, 1))
            ),
            (
                20,
                (d(2024, 2, 1), d(2024, 3, 1)),
                read(at(2024, 2, 1, 0), at(2024, 3, 1, 0)),
                recurring(d(2024, 3, 1))
            ),
            (
                20,
                (d(2024, 3, 1), d(2024, 4, 1)),
                read(at(2024, 3, 1, 0), at(2024, 4, 1, 0)),
                recurring(d(2024, 4, 1))
            ),
        ]
    );
    let needed: Vec<_> = run(&facts, now)
        .usage_needed
        .into_iter()
        .map(|need| (need.read, need.document))
        .collect();
    assert_eq!(
        needed,
        windows
            .iter()
            .map(|(_, _, read, document)| (*read, *document))
            .collect::<Vec<_>>()
    );
    assert!(
        usage_windows(&facts, now, &TestPricer)
            .unwrap()
            .iter()
            .all(|window| window.allowance == DayFraction::WHOLE)
    );
}

/// Cancelled on January 20 at 14:00, the window runs to the instant on the invoice the
/// cancellation opens; cancelled at 10:00 on February 1, a renewal, January stays on that
/// renewal's invoice and the morning is a window of that day alone on the final invoice, holding
/// none of a capacity's allowance.
/// Rules: R-016, R-002
#[test]
fn usage_windows_end_at_the_cancellation_on_its_document() {
    let mid = with_usage(Some(Ending {
        at: at(2024, 1, 20, 14),
        kind: EndingKind::Cancel,
    }));
    assert_eq!(
        windows_of(&mid, at(2024, 1, 25, 0)),
        vec![(
            20,
            (d(2024, 1, 1), d(2024, 1, 20)),
            read(at(2024, 1, 1, 0), at(2024, 1, 20, 14)),
            recurring(d(2024, 1, 20))
        )]
    );

    let renewal = with_usage(Some(Ending {
        at: at(2024, 2, 1, 10),
        kind: EndingKind::Cancel,
    }));
    assert_eq!(
        windows_of(&renewal, at(2024, 2, 2, 0)),
        vec![
            (
                20,
                (d(2024, 1, 1), d(2024, 2, 1)),
                read(at(2024, 1, 1, 0), at(2024, 2, 1, 0)),
                recurring(d(2024, 2, 1))
            ),
            (
                20,
                (d(2024, 2, 1), d(2024, 2, 1)),
                read(at(2024, 2, 1, 0), at(2024, 2, 1, 10)),
                DocumentSlot::Final
            ),
        ]
    );
    let morning = usage_windows(&renewal, at(2024, 2, 2, 0), &TestPricer).unwrap();
    assert_eq!(
        morning[1].allowance.prorate_decimal(Decimal::from(310)),
        Decimal::ZERO
    );
}

/// Switched to yearly on March 15, its origin April 1: the monthly fee's last window, March 1
/// to 15, rides the April 1 renewal; the yearly fee's stub, March 15 to April 1, ends on it; its
/// year from April 1 is open on the next year's invoice.
/// Rules: R-084, R-011
#[test]
fn usage_windows_follow_a_change_of_cadence() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts
        .plan_windows
        .push(plan(101, d(2024, 3, 15), 12, d(2024, 4, 1)));
    facts.fees.push(FeeFact {
        to: Some(d(2024, 3, 15)),
        ..metered(20, 2, d(2024, 1, 1), 1, 10)
    });
    facts.fees.push(metered(30, 3, d(2024, 3, 15), 12, 10));
    let windows: Vec<_> = windows_of(&facts, at(2024, 4, 10, 0))
        .into_iter()
        .map(|(row, days, _, document)| (row, days, document))
        .collect();
    assert_eq!(
        windows,
        vec![
            (20, (d(2024, 1, 1), d(2024, 2, 1)), recurring(d(2024, 2, 1))),
            (20, (d(2024, 2, 1), d(2024, 3, 1)), recurring(d(2024, 3, 1))),
            (
                20,
                (d(2024, 3, 1), d(2024, 3, 15)),
                recurring(d(2024, 4, 1))
            ),
            (
                30,
                (d(2024, 3, 15), d(2024, 4, 1)),
                recurring(d(2024, 4, 1))
            ),
            (30, (d(2024, 4, 1), d(2025, 4, 1)), recurring(d(2025, 4, 1))),
        ]
    );
}

/// A capacity of 310 units: a whole January holds all of them, cut on January 17 by a
/// cancellation 16 of 31 days, 160 units, and started on January 10 its first period's 22 of 31
/// days, 220 units, as its fee is prorated.
/// Rules: R-002
#[test]
fn a_usage_window_holds_the_share_of_a_capacitys_allowance_its_fee_bills() {
    let capacity = |start: NaiveDate| {
        let mut facts = subscription(start, 1, 1);
        facts.fees.push(FeeFact {
            billing: Billing::Advance,
            fee: TestFee {
                metric: Some(id(METRIC)),
                unit: 10,
                included: 310,
                per_period: true,
                ..TestFee::default()
            },
            ..metered(10, 1, start, 1, 10)
        });
        facts
    };
    let included = |facts: &Facts<TestFee>, now: DateTime<Utc>| -> Vec<Decimal> {
        usage_windows(facts, now, &TestPricer)
            .unwrap()
            .iter()
            .filter(|window| window.read.until <= now)
            .map(|window| window.allowance.prorate_decimal(Decimal::from(310)))
            .collect()
    };
    let whole = capacity(d(2024, 1, 1));
    assert_eq!(
        included(&whole, at(2024, 2, 1, 0)),
        vec![Decimal::from(310)]
    );

    let mut cancelled = whole.clone();
    cancelled.ending = Some(Ending {
        at: midnight(d(2024, 1, 17)),
        kind: EndingKind::Cancel,
    });
    assert_eq!(
        included(&cancelled, at(2024, 1, 18, 0)),
        vec![Decimal::from(160)]
    );

    let late = capacity(d(2024, 1, 10));
    assert_eq!(
        included(&late, at(2024, 3, 1, 0)),
        vec![Decimal::from(220), Decimal::from(310)]
    );
}

/// A watermark on March 15 leaves out the periods it settled, January and February; cleared,
/// they read whole again.
/// Rules: R-094
#[test]
fn usage_windows_leave_out_the_periods_a_watermark_settled() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(metered(20, 2, d(2024, 1, 1), 1, 10));
    facts.reconcile_from = Some(d(2024, 3, 15));
    let days = |facts: &Facts<TestFee>| -> Vec<(NaiveDate, NaiveDate)> {
        windows_of(facts, at(2024, 4, 10, 0))
            .into_iter()
            .map(|(_, days, _, _)| days)
            .collect()
    };
    assert_eq!(
        days(&facts),
        vec![
            (d(2024, 3, 1), d(2024, 4, 1)),
            (d(2024, 4, 1), d(2024, 5, 1))
        ]
    );
    facts.reconcile_from = None;
    assert_eq!(
        days(&facts),
        vec![
            (d(2024, 1, 1), d(2024, 2, 1)),
            (d(2024, 2, 1), d(2024, 3, 1)),
            (d(2024, 3, 1), d(2024, 4, 1)),
            (d(2024, 4, 1), d(2024, 5, 1))
        ]
    );
}

/// Ended the day it started with nothing billed in advance, the day's window opens that day's
/// invoice; a start not settled yet bills no window at all.
/// Rules: R-016, R-011
#[test]
fn usage_windows_follow_the_opening_invoice_rules() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(metered(20, 2, d(2024, 1, 1), 1, 10));
    let mut ended = facts.clone();
    ended.ending = Some(Ending {
        at: at(2024, 1, 1, 10),
        kind: EndingKind::Cancel,
    });
    assert_eq!(
        windows_of(&ended, at(2024, 1, 2, 0)),
        vec![(
            20,
            (d(2024, 1, 1), d(2024, 1, 1)),
            read(at(2024, 1, 1, 0), at(2024, 1, 1, 10)),
            recurring(d(2024, 1, 1))
        )]
    );

    facts.start.settled = None;
    assert!(windows_of(&facts, at(2024, 2, 10, 0)).is_empty());
}

// ---- properties

#[derive(Clone, Debug)]
struct Case {
    facts: Facts<TestFee>,
    t0: DateTime<Utc>,
    t1: DateTime<Utc>,
}

fn day(offset: u64) -> NaiveDate {
    d(2023, 1, 1) + Days::new(offset)
}

fn instant(offset_hours: u64) -> DateTime<Utc> {
    midnight(d(2023, 1, 1)) + chrono::Duration::hours(i64::try_from(offset_hours).unwrap())
}

type FeeSpec = (u32, bool, i64, u64, Option<u64>, bool);

fn fee_spec() -> impl Strategy<Value = FeeSpec> {
    (
        prop::sample::select(vec![1u32, 3, 12]),
        any::<bool>(),
        0i64..50_000,
        0u64..200,
        prop::option::of(1u64..300),
        prop::bool::weighted(0.3),
    )
}

prop_compose! {
    fn case()(
        start in 0u64..200,
        anchor in 1u32..=31,
        cadence in prop::sample::select(vec![1u32, 3, 12]),
        trial in prop::option::of((1u64..30, any::<bool>())),
        settled in prop::bool::weighted(0.9),
        fees in prop::collection::vec(fee_spec(), 1..4),
        override_row in prop::option::of((1u64..200, 0i64..50_000)),
        steps in prop::collection::vec((0u64..300, 0u64..20), 0..3),
        switch in prop::option::of((30u64..300, prop::sample::select(vec![1u32, 3, 12]))),
        ending in prop::option::of((0u64..400, 0u64..24, any::<bool>(), any::<bool>())),
        end_date in prop::option::of(30u64..500),
        minimum in prop::option::of(0i64..20_000),
        watermark in prop::option::of(0u64..500),
        t0 in 0u64..(600 * 24),
        gap in 1u64..(400 * 24),
    ) -> Case {
        let start_date = day(start);
        let mut facts = subscription(start_date, anchor, cadence);
        if !settled {
            facts.start.settled = None;
            facts.start.checkout = true;
        }
        facts.trial = trial.map(|(days, free)| Trial {
            from: start_date,
            until: start_date + Days::new(days),
            free,
        });
        let billing_start = facts.billing_start();
        let old = if facts.bills_from_trial_end() {
            Grid::from_trial_end(billing_start, anchor, cadence)
        } else {
            Grid::from_start(billing_start, anchor, cadence)
        }
        .unwrap();
        if let Some((offset, after)) = switch {
            let on = start_date + Days::new(offset);
            // R-084: the change's first renewal, on the grid in force before it or sooner on its own.
            let origin = if after == cadence {
                start_date
            } else if old.is_boundary(on) {
                on
            } else {
                let next = old.period_containing(on).unwrap().1;
                Grid::from_origin(next, anchor, after)
                    .unwrap()
                    .period_containing(on)
                    .unwrap()
                    .1
            };
            facts.plan_windows[0].to = Some(on);
            facts.plan_windows.push(plan(101, on, after, origin));
        }
        for (n, (fee_cadence, arrears, price, from, life, usage)) in fees.into_iter().enumerate() {
            let n = n as u128;
            let from = start_date + Days::new(from);
            let mut row = if usage {
                metered(10 + n, 1 + n, from, fee_cadence, 1 + price % 100)
            } else {
                fixed(10 + n, 1 + n, from, fee_cadence, price)
            };
            if arrears {
                row.billing = Billing::Arrears;
            }
            row.to = life.map(|days| from + Days::new(days));
            facts.fees.push(row);
        }
        if let Some((offset, price)) = override_row {
            let first = facts.fees[0].clone();
            facts.fees.push(FeeFact {
                id: id(99),
                from: first.from + Days::new(offset),
                fee: TestFee { price, ..first.fee.clone() },
                ..first
            });
        }
        facts.seats = steps
            .into_iter()
            .enumerate()
            .map(|(n, (offset, quantity))| SeatStep {
                id: id(500 + n as u128),
                lineage: lineage(1),
                at: start_date + Days::new(offset),
                quantity,
                removed_at_renewal: 0,
            })
            .collect();
        facts.ending = ending.map(|(offset, hour, cancel, on_renewal)| {
            let mut on = start_date + Days::new(offset);
            if on_renewal {
                on = old.period_containing(on).unwrap().1;
            }
            Ending {
                at: midnight(on) + chrono::Duration::hours(hour as i64),
                kind: if cancel { EndingKind::Cancel } else { EndingKind::Complete },
            }
        });
        facts.end_date = end_date.map(|offset| start_date + Days::new(offset));
        facts.reconcile_from = watermark.map(|offset| start_date + Days::new(offset));
        facts.minimums = minimum
            .map(|amount| MinimumFact {
                id: id(700),
                from: start_date,
                to: None,
                amount,
                scope: MinimumScope::AllComponents,
            })
            .into_iter()
            .collect();
        Case {
            facts,
            t0: instant(t0),
            t1: instant(t0 + gap),
        }
    }
}

fn fee_charges(evaluation: &Evaluation) -> Vec<&Charge> {
    evaluation
        .charges
        .iter()
        .filter(|charge| charge.usage.is_none())
        .collect()
}

/// Every read the facts need at `now`, each with a quantity of its own.
fn all_reads(facts: &Facts<TestFee>, now: DateTime<Utc>) -> UsageReads {
    run(facts, now)
        .usage_needed
        .into_iter()
        .map(|need| {
            let quantity = (need.read.window.from.timestamp() / 3_600
                + need.read.window.until.timestamp() / 7_200)
                .rem_euclid(50);
            (need.read, vec![(BTreeMap::new(), Decimal::from(quantity))])
        })
        .collect()
}

proptest! {
    #[test]
    fn the_same_facts_and_reads_give_the_same_evaluation(case in case()) {
        let usage = all_reads(&case.facts, case.t1);
        let first = evaluate(&case.facts, case.t1, Some(&usage), &TestPricer).unwrap();
        let second = evaluate(&case.facts, case.t1, Some(&usage), &TestPricer).unwrap();
        prop_assert_eq!(first, second);
    }

    #[test]
    fn fee_charges_due_by_an_instant_never_change_later(case in case()) {
        // An ending before the start settles takes back the checkout's invoice (R-013).
        prop_assume!(
            case.facts.start.settled.is_some()
                || case.facts.ending.is_none_or(|ending| ending.at > case.t1)
        );
        let early = run(&case.facts, case.t0);
        let late = run(&case.facts, case.t1);
        let due: Vec<&Charge> = fee_charges(&late)
            .into_iter()
            .filter(|charge| charge.due_at <= case.t0)
            .collect();
        prop_assert_eq!(fee_charges(&early), due);
        prop_assert!(early.charges.iter().all(|charge| charge.due_at <= case.t0));
    }

    #[test]
    fn nothing_falls_due_before_the_next_wake(case in case()) {
        let evaluation = run(&case.facts, case.t0);
        if let Some(wake) = evaluation.next_wake {
            prop_assert!(wake > case.t0);
            let before = case.t0 + (wake - case.t0) / 2;
            let later = run(&case.facts, before);
            prop_assert_eq!(fee_charges(&evaluation), fee_charges(&later));
        }
    }

    /// Usage included: an ending's line of no days bills only its own key (R-016, R-080).
    #[test]
    fn reconcile_after_applying_an_evaluation_gives_nothing(case in case()) {
        let usage = all_reads(&case.facts, case.t1);
        let early = evaluate(&case.facts, case.t0, Some(&usage), &TestPricer).unwrap();
        let late = evaluate(&case.facts, case.t1, Some(&usage), &TestPricer).unwrap();
        let mut lines = issue(&early.expected());
        let expected = late.expected();
        let adjustments = reconcile(&expected, &lines, TakeBack::LatestFirst);
        let short = adjustments.iter().any(|a| matches!(a, Adjustment::Short { .. }));
        prop_assert!(!short);
        apply(&mut lines, &adjustments);
        prop_assert!(reconcile(&expected, &lines, TakeBack::LatestFirst).is_empty());
    }

    /// A watermark leaves out what it settled and nothing else: the charges keyed from it on
    /// are the same without it, and a period straddling it is still computed.
    #[test]
    fn a_watermark_changes_no_charge_keyed_from_it(case in case(), mark in 0u64..700) {
        let watermark = day(mark);
        let mut open = case.facts.clone();
        open.reconcile_from = None;
        let mut marked = case.facts.clone();
        marked.reconcile_from = Some(watermark);
        let usage = all_reads(&open, case.t1);
        let without = evaluate(&open, case.t1, Some(&usage), &TestPricer).unwrap();
        let with = evaluate(&marked, case.t1, Some(&usage), &TestPricer).unwrap();
        // What rode a renewal the watermark settled rides the first one past it instead, or the
        // ending's document.
        let moved = |at: DocumentSlot, to: DocumentSlot| match (at, to) {
            _ if at == to => true,
            (DocumentSlot::Recurring { period_start: a }, DocumentSlot::Final) => a <= watermark,
            (
                DocumentSlot::Recurring { period_start: a },
                DocumentSlot::Recurring { period_start: b },
            ) => a <= watermark && b > a && b >= watermark,
            _ => false,
        };
        let unplaced = |charge: &Charge| Charge {
            document: DocumentSlot::Final,
            ..charge.clone()
        };
        let from_mark = |evaluation: &Evaluation| -> Vec<Charge> {
            evaluation
                .charges
                .iter()
                .filter(|charge| charge.key.from >= watermark)
                .map(unplaced)
                .collect()
        };
        prop_assert_eq!(from_mark(&with), from_mark(&without));
        let placed_alike = |charge: &Charge, among: &[Charge]| {
            among
                .iter()
                .any(|other| unplaced(other) == unplaced(charge) && moved(other.document, charge.document))
        };
        for charge in &with.charges {
            prop_assert!(placed_alike(charge, &without.charges), "{:?}", charge);
            if charge.usage.is_none() {
                prop_assert!(charge.key.until > watermark, "{:?}", charge);
            }
        }
        for charge in &without.charges {
            if charge.usage.is_none() && charge.key.until > watermark {
                prop_assert!(
                    with.charges.iter().any(|other| unplaced(other) == unplaced(charge)
                        && moved(charge.document, other.document)),
                    "{:?}",
                    charge
                );
            }
        }
        for need in &with.usage_needed {
            prop_assert!(without.usage_needed.iter().any(|other| other.fee == need.fee
                && other.read == need.read
                && moved(other.document, need.document)));
        }
        prop_assert_eq!(with.state, without.state);
    }

    #[test]
    fn a_keys_charges_never_add_up_below_zero(case in case()) {
        let evaluation = run(&case.facts, case.t1);
        for expected in evaluation.expected() {
            prop_assert!(expected.amount >= 0, "{:?}", expected);
        }
    }
}

#[test]
fn a_price_out_of_range_fails_the_evaluation() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    let mut row = fixed(10, 1, d(2024, 1, 1), 1, 3_100);
    row.fee.overflows = true;
    facts.fees.push(row);
    assert_eq!(
        evaluate(&facts, at(2024, 1, 10, 0), None, &TestPricer),
        Err(PricingError::OutOfRange)
    );
    let mut metered_facts = subscription(d(2024, 1, 1), 1, 1);
    let mut row = metered(20, 2, d(2024, 1, 1), 1, 5);
    row.fee.overflows = true;
    metered_facts.fees.push(row);
    let usage = reads(&[(read(at(2024, 1, 1, 0), at(2024, 2, 1, 0)), 10)]);
    assert_eq!(
        evaluate(&metered_facts, at(2024, 2, 2, 0), Some(&usage), &TestPricer),
        Err(PricingError::OutOfRange)
    );
}

/// The trial's 17 of January's 31 days: the pack grants that share of its credit, and names the
/// product its once rule is read for.
#[test]
fn a_packs_grant_takes_its_charges_share() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.trial = Some(Trial {
        from: d(2024, 1, 1),
        until: d(2024, 1, 15),
        free: true,
    });
    let mut row = fixed(10, 1, d(2024, 1, 1), 1, 3_100);
    row.fee.grant = Some(6_200);
    row.fee.once_product = Some(id(500));
    facts.fees.push(row);
    let evaluation = run(&facts, at(2024, 1, 20, 0));
    let charges = of(&evaluation, 1);
    assert_eq!(charges[0].amount, 1_700);
    assert_eq!(
        charges[0].credit_grant,
        Some(CreditGrant {
            amount: 3_400,
            denomination: Denomination::Money,
            prorated: true,
        })
    );
    assert_eq!(charges[0].once_per_product, Some(id(500)));
}

/// A fee priced in money, then in credits from the 16th: two charges of the period, never
/// netted against each other.
#[test]
fn money_and_credits_are_charged_apart() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    let mut money = fixed(10, 1, d(2024, 1, 1), 1, 3_100);
    money.to = Some(d(2024, 1, 16));
    let mut credits = fixed(11, 1, d(2024, 1, 16), 1, 3_100);
    credits.fee.credit_unit = Some(id(600));
    facts.fees.extend([money, credits]);
    let expected = run(&facts, at(2024, 1, 20, 0)).expected();
    let mut denominations: Vec<Denomination> = expected
        .iter()
        .filter(|e| e.key.fee == lineage(1) && e.key.from == d(2024, 1, 1))
        .map(|e| e.key.denomination)
        .collect();
    denominations.sort();
    assert_eq!(
        denominations,
        vec![Denomination::Money, Denomination::CreditUnit(id(600))]
    );
    let in_credits = expected
        .iter()
        .find(|e| e.key.denomination == Denomination::CreditUnit(id(600)))
        .unwrap();
    assert_eq!(in_credits.amount, 1_600);
}
