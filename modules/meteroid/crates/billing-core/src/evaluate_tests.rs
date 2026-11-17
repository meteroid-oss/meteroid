use std::collections::BTreeMap;

use chrono::{DateTime, Days, NaiveDate, Utc};
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
    metric: Option<Uuid>,
    /// Per unit of usage.
    unit: i64,
    /// Priced in credits of this unit.
    credit_unit: Option<Uuid>,
    /// A pack's credit for a whole period, per seat.
    grant: Option<i64>,
    once_product: Option<Uuid>,
    /// Its price is out of range.
    overflows: bool,
}

impl TestFee {
    fn denomination(&self) -> Denomination {
        self.credit_unit
            .map_or(Denomination::Money, Denomination::CreditUnit)
    }
}

struct TestPricer;

impl Pricer<TestFee> for TestPricer {
    fn metrics(&self, fee: &TestFee) -> Vec<Uuid> {
        fee.metric.into_iter().collect()
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
                    kind: ChargeKind::Fee,
                    quantity: Some(Decimal::from(seats)),
                    amount: fee.fee.price * seats,
                    denomination: fee.fee.denomination(),
                    prorated: true,
                    credit_grant: fee.fee.grant.map(|grant| CreditGrant {
                        amount: grant * seats,
                        denomination: Denomination::Money,
                        prorated: true,
                    }),
                    once_per_product: fee.fee.once_product,
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
                Some(Ok(Rate {
                    fee: fee.lineage.clone(),
                    kind: ChargeKind::Usage {
                        metric,
                        dimensions: BTreeMap::new(),
                    },
                    quantity: Some(total),
                    amount: (total * Decimal::from(fee.fee.unit)).to_i64().unwrap(),
                    denomination: fee.fee.denomination(),
                    prorated: false,
                    credit_grant: None,
                    once_per_product: None,
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

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
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

#[test]
fn a_free_trial_ended_before_its_checkout_settled_expires() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.start.settled = None;
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

#[test]
fn a_checkout_start_not_settled_is_pending_and_bills_nothing() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.start.settled = None;
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1, 3_100));
    let evaluation = run(&facts, at(2024, 1, 10, 0));
    assert!(evaluation.charges.is_empty());
    assert_eq!(evaluation.state.status, Status::Pending);
    assert!(evaluation.state.pending_checkout);

    let mut later = subscription(d(2024, 3, 1), 1, 1);
    later.fees.push(fixed(10, 1, d(2024, 3, 1), 1, 3_100));
    let before = run(&later, at(2024, 2, 1, 0));
    assert_eq!(before.state.status, Status::Pending);
    assert!(!before.state.pending_checkout);
    assert_eq!(before.next_wake, Some(at(2024, 3, 1, 0)));
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
            read: window
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
            read: read(at(2024, 3, 1, 0), at(2024, 4, 1, 0))
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
                read: read(at(2024, 3, 1, 0), at(2024, 4, 1, 0))
            },
            UsageNeed {
                fee: lineage(2),
                read: read(at(2024, 4, 1, 0), at(2024, 5, 1, 0))
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
            read: window
        }]
    );
    assert_eq!(evaluation.next_wake, Some(at(2024, 2, 1, 0)));
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
        }
        facts.trial = trial.map(|(days, free)| Trial {
            from: start_date,
            until: start_date + Days::new(days),
            free,
        });
        let billing_start = facts.billing_start().unwrap_or(start_date);
        let old = if facts.bills_from_trial_end() {
            Grid::from_trial_end(billing_start, anchor, cadence)
        } else {
            Grid::from_start(billing_start, anchor, cadence)
        }
        .unwrap();
        if let Some((offset, after)) = switch {
            let on = start_date + Days::new(offset);
            // R-084: the first boundary on or after the change of the grid in force before it.
            let origin = if after == cadence {
                start_date
            } else if old.is_boundary(on) {
                on
            } else {
                old.period_containing(on).unwrap().1
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
        let from_mark = |evaluation: &Evaluation| -> Vec<Charge> {
            evaluation
                .charges
                .iter()
                .filter(|charge| charge.key.from >= watermark)
                .cloned()
                .collect()
        };
        prop_assert_eq!(from_mark(&with), from_mark(&without));
        for charge in &with.charges {
            prop_assert!(without.charges.contains(charge));
            if charge.usage.is_none() {
                prop_assert!(charge.key.until > watermark, "{:?}", charge);
            }
        }
        for charge in &without.charges {
            if charge.usage.is_none() && charge.key.until > watermark {
                prop_assert!(with.charges.contains(charge), "{:?}", charge);
            }
        }
        for need in &with.usage_needed {
            prop_assert!(without.usage_needed.contains(need));
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
