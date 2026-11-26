use std::collections::BTreeSet;

use rust_decimal::Decimal;
use uuid::Uuid;

use super::*;
use crate::charge::{ChargeKind, Denomination, Fee};
use crate::facts::{Billing, Ending, EndingKind, FeeFact, PlanWindow, Start, Trial};

#[derive(Clone, Debug, Default, PartialEq)]
struct TestFee {
    /// A whole period's price.
    price: i64,
}

struct TestPricer;

impl Pricer<TestFee> for TestPricer {
    fn metrics(&self, _: &TestFee) -> Vec<common_domain::ids::BillableMetricId> {
        Vec::new()
    }

    fn rates(&self, segment: &Segment<'_, TestFee>) -> Result<Vec<Rate>, PricingError> {
        Ok(segment
            .fees
            .iter()
            .map(|fee| Rate {
                fee: fee.lineage.clone(),
                kind: ChargeKind::Fee,
                quantity: Some(Decimal::ONE),
                amount: fee.fee.price,
                denomination: Denomination::Money,
                prorated: true,
                credit_grant: None,
                once_per_product: None,
                shown: Default::default(),
            })
            .collect())
    }

    fn usage(
        &self,
        _: &Segment<'_, TestFee>,
        _: &UsageWindow,
        _: &UsageReads,
    ) -> Result<Vec<Rate>, PricingError> {
        Ok(Vec::new())
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

fn plan(version: u128, from: NaiveDate, cadence: u32, origin: NaiveDate) -> PlanWindow {
    PlanWindow {
        plan_version: id(version),
        from,
        to: None,
        cadence,
        cadence_origin: origin,
    }
}

/// Started on `start` with no checkout (settled then), on plan version 100.
fn subscription(start: NaiveDate, anchor: u32, cadence: u32) -> Facts<TestFee> {
    Facts {
        subscription: id(1),
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

fn fee(row: u128, of: u128, from: NaiveDate, billing: Billing, price: i64) -> FeeFact<TestFee> {
    FeeFact {
        id: id(row),
        lineage: Fee::Component(id(of)),
        from,
        to: None,
        billing,
        cadence: 1,
        plan_component: true,
        usage_from: None,
        fee: TestFee { price },
    }
}

fn fixed(row: u128, of: u128, from: NaiveDate, price: i64) -> FeeFact<TestFee> {
    fee(row, of, from, Billing::Advance, price)
}

fn run(facts: &Facts<TestFee>, now: DateTime<Utc>) -> Evaluation {
    evaluate(facts, now, None, &TestPricer).unwrap()
}

fn no_deadlines() -> Deadlines<'static> {
    Deadlines {
        finalize_after: &[],
        next_payment_attempt_at: &[],
        credit_lapses: &[],
        settlement_grace: Duration::zero(),
    }
}

fn recurring(on: NaiveDate) -> DocumentSlot {
    DocumentSlot::Recurring { period_start: on }
}

fn slots(step: &Step) -> Vec<DocumentSlot> {
    step.documents
        .iter()
        .map(|document| document.slot)
        .collect()
}

fn status(from: Status, to: Status, at: DateTime<Utc>) -> Transition {
    Transition::Status { from, to, at }
}

/// What the controller stored for a subscription and the documents it wrote.
struct Controller {
    stored: Stored,
    written: Vec<DocumentSlot>,
}

impl Controller {
    /// As the insert leaves it: the state its facts give at `at`, nothing written.
    fn inserted(facts: &Facts<TestFee>, at: DateTime<Utc>) -> Self {
        Controller {
            stored: Stored::from(&run(facts, at).state),
            written: Vec::new(),
        }
    }

    /// As a controller up to date at `at` leaves it.
    fn caught_up(facts: &Facts<TestFee>, at: DateTime<Utc>) -> Self {
        let evaluation = run(facts, at);
        Controller {
            stored: Stored::from(&evaluation.state),
            written: evaluation
                .charges
                .iter()
                .map(|charge| charge.document)
                .collect(),
        }
    }

    fn step(&self, facts: &Facts<TestFee>, now: DateTime<Utc>) -> Step {
        let evaluation = run(facts, now);
        step(
            facts,
            now,
            &evaluation,
            &self.stored,
            &self.written,
            &no_deadlines(),
        )
    }

    /// The step at `now`, written.
    fn apply(&mut self, facts: &Facts<TestFee>, now: DateTime<Utc>) -> Step {
        let step = self.step(facts, now);
        if let Some(state) = step.state {
            self.stored = state;
        }
        self.written
            .extend(step.documents.iter().map(|document| document.slot));
        step
    }
}

/// Rules: R-084
#[test]
fn a_subscription_activates_on_its_start_date() {
    let mut facts = subscription(d(2024, 3, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 3, 1), 3_100));
    let mut controller = Controller::inserted(&facts, at(2024, 2, 1, 0));
    assert_eq!(controller.stored.status, Status::Pending);

    let before = controller.apply(&facts, at(2024, 2, 15, 0));
    assert!(before.is_empty());
    assert_eq!(before.next_wake_at, Some(at(2024, 3, 1, 0)));

    let started = controller.apply(&facts, at(2024, 3, 1, 1));
    assert_eq!(
        started.transitions,
        vec![status(Status::Pending, Status::Active, at(2024, 3, 1, 0))]
    );
    let state = started.state.unwrap();
    assert_eq!(state.status, Status::Active);
    assert_eq!(
        state.current_period,
        Some((d(2024, 3, 1), Some(d(2024, 4, 1))))
    );
    assert_eq!(slots(&started), vec![recurring(d(2024, 3, 1))]);
    assert_eq!(started.documents[0].at, at(2024, 3, 1, 0));
    assert_eq!(started.documents[0].charges[0].amount, 3_100);
    assert!(!started.documents[0].awaiting_checkout);
    assert_eq!(started.next_wake_at, Some(at(2024, 4, 1, 0)));
    assert!(controller.step(&facts, at(2024, 3, 1, 1)).is_empty());
}

/// A checkout start not settled stays pending, with its opening invoice alone, the checkout's;
/// settled, it activates at the settlement and writes the renewals it waited for.
/// Rules: R-011
#[test]
fn a_checkout_start_still_pending_writes_its_opening_invoice_alone() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.start.settled = None;
    facts.start.checkout = true;
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 3_100));
    let mut controller = Controller::inserted(&facts, at(2023, 12, 15, 0));

    let pending = controller.apply(&facts, at(2024, 3, 10, 0));
    assert!(pending.transitions.is_empty());
    let state = pending.state.unwrap();
    assert_eq!(state.status, Status::Pending);
    assert!(state.pending_checkout);
    assert_eq!(state.current_period, Some((d(2024, 1, 1), None)));
    assert_eq!(slots(&pending), vec![recurring(d(2024, 1, 1))]);
    assert!(pending.documents[0].awaiting_checkout);
    assert!(controller.step(&facts, at(2024, 3, 10, 0)).is_empty());

    facts.start.settled = Some(at(2024, 3, 10, 12));
    let paid = controller.apply(&facts, at(2024, 3, 10, 13));
    assert_eq!(
        paid.transitions,
        vec![status(Status::Pending, Status::Active, at(2024, 3, 10, 12))]
    );
    assert!(!paid.state.unwrap().pending_checkout);
    assert_eq!(
        slots(&paid),
        vec![recurring(d(2024, 2, 1)), recurring(d(2024, 3, 1))]
    );
    assert!(paid.documents.iter().all(|due| !due.awaiting_checkout));
}

/// A free trial bills nothing; at its end, its start settled, the subscription is active and
/// its first period, from the trial's end, is due.
/// Rules: R-084, OQ9
#[test]
fn a_free_trial_ends_active_when_its_start_is_settled() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.trial = Some(Trial {
        from: d(2024, 1, 1),
        until: d(2024, 1, 15),
        free: true,
    });
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 3_100));
    let mut controller = Controller::inserted(&facts, at(2023, 12, 15, 0));

    let trialing = controller.apply(&facts, at(2024, 1, 1, 1));
    assert_eq!(
        trialing.transitions,
        vec![status(Status::Pending, Status::Trialing, at(2024, 1, 1, 0))]
    );
    assert_eq!(
        trialing.state.unwrap().current_period,
        Some((d(2024, 1, 1), Some(d(2024, 1, 15))))
    );
    assert!(trialing.documents.is_empty());
    assert_eq!(trialing.next_wake_at, Some(at(2024, 1, 15, 0)));

    let ended = controller.apply(&facts, at(2024, 1, 15, 1));
    assert_eq!(
        ended.transitions,
        vec![status(Status::Trialing, Status::Active, at(2024, 1, 15, 0))]
    );
    assert_eq!(
        ended.state.unwrap().current_period,
        Some((d(2024, 1, 15), Some(d(2024, 2, 1))))
    );
    assert_eq!(slots(&ended), vec![recurring(d(2024, 1, 15))]);
    assert_eq!(ended.documents[0].charges[0].amount, 1_700);
}

/// Ended before its checkout settled, a free trial expires: its first period is the checkout's
/// invoice. Settled later, it activates then and writes the renewals it waited for.
/// Rules: R-084, R-011
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
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 3_100));
    let mut controller = Controller::inserted(&facts, at(2023, 12, 15, 0));
    controller.apply(&facts, at(2024, 1, 10, 0));
    assert_eq!(controller.stored.status, Status::Trialing);

    let expired = controller.apply(&facts, at(2024, 1, 20, 0));
    assert_eq!(
        expired.transitions,
        vec![status(
            Status::Trialing,
            Status::TrialExpired,
            at(2024, 1, 15, 0)
        )]
    );
    assert!(expired.state.unwrap().pending_checkout);
    assert_eq!(slots(&expired), vec![recurring(d(2024, 1, 15))]);
    assert!(expired.documents[0].awaiting_checkout);
    assert!(
        controller
            .step(&facts, at(2024, 3, 20, 0))
            .transitions
            .is_empty()
    );

    facts.start.settled = Some(at(2024, 4, 10, 0));
    let paid = controller.apply(&facts, at(2024, 4, 10, 1));
    assert_eq!(
        paid.transitions,
        vec![status(
            Status::TrialExpired,
            Status::Active,
            at(2024, 4, 10, 0)
        )]
    );
    assert!(!paid.state.unwrap().pending_checkout);
    assert_eq!(
        slots(&paid),
        vec![
            recurring(d(2024, 2, 1)),
            recurring(d(2024, 3, 1)),
            recurring(d(2024, 4, 1)),
        ]
    );
}

/// A paid trial bills from its start like any period; its end changes the status alone.
/// Rules: R-084
#[test]
fn a_paid_trial_bills_from_its_start_and_ends_by_its_status() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.trial = Some(Trial {
        from: d(2024, 1, 1),
        until: d(2024, 1, 15),
        free: false,
    });
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 3_100));
    let mut controller = Controller::inserted(&facts, at(2023, 12, 15, 0));

    let started = controller.apply(&facts, at(2024, 1, 1, 1));
    assert_eq!(
        started.transitions,
        vec![status(Status::Pending, Status::Trialing, at(2024, 1, 1, 0))]
    );
    assert_eq!(
        started.state.unwrap().current_period,
        Some((d(2024, 1, 1), Some(d(2024, 2, 1))))
    );
    assert_eq!(slots(&started), vec![recurring(d(2024, 1, 1))]);
    assert_eq!(started.documents[0].charges[0].amount, 3_100);

    let ended = controller.apply(&facts, at(2024, 1, 15, 1));
    assert_eq!(
        ended.transitions,
        vec![status(Status::Trialing, Status::Active, at(2024, 1, 15, 0))]
    );
    let state = ended.state.unwrap();
    assert_eq!(state.status, Status::Active);
    assert_eq!(
        state.current_period,
        Some((d(2024, 1, 1), Some(d(2024, 2, 1))))
    );
    assert!(ended.documents.is_empty());
    assert_eq!(ended.next_wake_at, Some(at(2024, 2, 1, 0)));
}

/// At a boundary the step stores the new period and writes its invoice; a controller that ran
/// late writes each renewal it missed.
/// Rules: R-084, R-051
#[test]
fn a_renewal_at_a_boundary_stores_its_period_and_writes_its_invoice() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 3_100));
    let mut controller = Controller::inserted(&facts, at(2023, 12, 15, 0));
    controller.apply(&facts, at(2024, 1, 1, 1));
    let mut late = Controller {
        stored: controller.stored,
        written: controller.written.clone(),
    };
    assert!(controller.apply(&facts, at(2024, 1, 20, 0)).is_empty());

    let renewal = controller.apply(&facts, at(2024, 2, 1, 1));
    assert!(renewal.transitions.is_empty());
    assert_eq!(
        renewal.state.unwrap().current_period,
        Some((d(2024, 2, 1), Some(d(2024, 3, 1))))
    );
    assert_eq!(slots(&renewal), vec![recurring(d(2024, 2, 1))]);
    assert_eq!(renewal.documents[0].at, at(2024, 2, 1, 0));
    assert_eq!(renewal.next_wake_at, Some(at(2024, 3, 1, 0)));

    let caught_up = late.apply(&facts, at(2024, 4, 1, 1));
    assert!(caught_up.transitions.is_empty());
    assert_eq!(
        slots(&caught_up),
        vec![
            recurring(d(2024, 2, 1)),
            recurring(d(2024, 3, 1)),
            recurring(d(2024, 4, 1)),
        ]
    );
}

/// A cancellation still to come changes nothing until it applies, the controller waking for it;
/// applied, the subscription is cancelled and the arrears it cut are due on its date.
/// Rules: R-016
#[test]
fn a_cancellation_applies_at_its_instant() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 3_100));
    facts
        .fees
        .push(fee(20, 2, d(2024, 1, 1), Billing::Arrears, 3_100));
    facts.ending = Some(Ending {
        at: at(2024, 1, 20, 0),
        kind: EndingKind::Cancel,
    });
    let mut controller = Controller::inserted(&facts, at(2023, 12, 15, 0));
    controller.apply(&facts, at(2024, 1, 1, 1));
    assert_eq!(controller.written, vec![recurring(d(2024, 1, 1))]);

    let waiting = controller.apply(&facts, at(2024, 1, 10, 0));
    assert!(waiting.is_empty());
    assert_eq!(waiting.next_wake_at, Some(at(2024, 1, 20, 0)));

    let cancelled = controller.apply(&facts, at(2024, 1, 21, 0));
    assert_eq!(
        cancelled.transitions,
        vec![status(
            Status::Active,
            Status::Cancelled,
            at(2024, 1, 20, 0)
        )]
    );
    let state = cancelled.state.unwrap();
    assert_eq!(state.status, Status::Cancelled);
    assert_eq!(state.current_period, Some((d(2024, 1, 20), None)));
    assert_eq!(slots(&cancelled), vec![recurring(d(2024, 1, 20))]);
    assert_eq!(cancelled.next_wake_at, None);
    assert!(controller.step(&facts, at(2024, 1, 21, 0)).is_empty());
}

/// Rules: R-013
#[test]
fn a_cancellation_before_the_start_settled_bills_nothing() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.start.settled = None;
    facts.start.checkout = true;
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 3_100));
    facts.ending = Some(Ending {
        at: at(2024, 1, 5, 12),
        kind: EndingKind::Cancel,
    });
    let mut controller = Controller::inserted(&facts, at(2023, 12, 15, 0));

    let cancelled = controller.apply(&facts, at(2024, 1, 10, 0));
    assert_eq!(
        cancelled.transitions,
        vec![status(
            Status::Pending,
            Status::Cancelled,
            at(2024, 1, 5, 12)
        )]
    );
    assert_eq!(cancelled.state.unwrap().status, Status::Cancelled);
    assert!(cancelled.documents.is_empty());
}

/// A fixed term completes the subscription at its end date's midnight, nothing left to bill.
/// Rules: R-084, R-016
#[test]
fn a_fixed_term_completes_at_its_end() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 1_000));
    facts.end_date = Some(d(2024, 3, 1));
    let mut controller = Controller::caught_up(&facts, at(2024, 2, 15, 0));
    assert_eq!(controller.stored.status, Status::Active);

    let completed = controller.apply(&facts, at(2024, 3, 1, 1));
    assert_eq!(
        completed.transitions,
        vec![status(Status::Active, Status::Completed, at(2024, 3, 1, 0))]
    );
    let state = completed.state.unwrap();
    assert_eq!(state.status, Status::Completed);
    assert_eq!(state.current_period, Some((d(2024, 3, 1), None)));
    assert!(completed.documents.is_empty());
    assert_eq!(completed.next_wake_at, None);
}

/// A monthly plan switched to a yearly one on July 1, the change's origin: the step stores the
/// new version, its cadence and its first year, and writes the renewal billing it.
/// Rules: R-084
#[test]
fn a_scheduled_plan_window_change_applies_at_its_origin() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts
        .plan_windows
        .push(plan(101, d(2024, 7, 1), 12, d(2024, 7, 1)));
    facts.fees.push(FeeFact {
        to: Some(d(2024, 7, 1)),
        ..fixed(10, 1, d(2024, 1, 1), 1_000)
    });
    facts.fees.push(FeeFact {
        cadence: 12,
        ..fixed(20, 2, d(2024, 7, 1), 12_000)
    });
    let mut controller = Controller::caught_up(&facts, at(2024, 6, 15, 0));
    assert_eq!(controller.stored.plan_version, Some(id(100)));

    let changed = controller.apply(&facts, at(2024, 7, 1, 1));
    assert_eq!(
        changed.transitions,
        vec![Transition::Plan {
            from: id(100),
            to: id(101),
            at: at(2024, 7, 1, 0),
        }]
    );
    let state = changed.state.unwrap();
    assert_eq!(state.plan_version, Some(id(101)));
    assert_eq!(state.cadence, Some(12));
    assert_eq!(
        state.current_period,
        Some((d(2024, 7, 1), Some(d(2025, 7, 1))))
    );
    assert_eq!(slots(&changed), vec![recurring(d(2024, 7, 1))]);
    assert_eq!(changed.documents[0].charges[0].amount, 12_000);
    assert!(controller.step(&facts, at(2024, 7, 1, 1)).is_empty());
}

/// A monthly plan changed for Feb 1 to another, stored at that renewal, changed back the same
/// day: the window of the change back took the place of the one the change opened. The plan
/// changes back on Feb 1, where it read the run of the first plan from Jan 1, a second switch
/// dated the start that cut the trial's end there for its readers.
/// Rules: R-084, R-100
#[test]
fn a_plan_changed_back_the_day_its_window_opened_changes_back_that_day() {
    let mut facts = subscription(d(2025, 1, 1), 1, 1);
    facts.plan_windows = vec![
        PlanWindow {
            to: Some(d(2025, 2, 1)),
            ..plan(100, d(2025, 1, 1), 1, d(2025, 1, 1))
        },
        plan(101, d(2025, 2, 1), 1, d(2025, 2, 1)),
    ];
    facts.fees.push(FeeFact {
        to: Some(d(2025, 2, 1)),
        ..fixed(10, 1, d(2025, 1, 1), 1_000)
    });
    facts.fees.push(fixed(20, 2, d(2025, 2, 1), 2_000));
    let controller = Controller::caught_up(&facts, at(2025, 2, 1, 1));
    assert_eq!(controller.stored.plan_version, Some(id(101)));

    facts.plan_windows[1] = plan(100, d(2025, 2, 1), 1, d(2025, 2, 1));
    facts.fees[1].to = Some(d(2025, 2, 1));
    facts.fees.push(fixed(30, 1, d(2025, 2, 1), 1_000));
    assert_eq!(
        controller.step(&facts, at(2025, 2, 1, 16)).transitions,
        vec![Transition::Plan {
            from: id(101),
            to: id(100),
            at: at(2025, 2, 1, 0),
        }]
    );
}

/// Run daily or once late, the controller emits the same transitions in the same order and ends
/// with the same state and documents; run again at the same instant, a step is empty.
/// Rules: R-084, R-011
#[test]
fn the_step_is_idempotent_and_the_same_run_daily_or_late() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.start.settled = Some(at(2024, 2, 10, 12));
    facts.trial = Some(Trial {
        from: d(2024, 1, 1),
        until: d(2024, 1, 15),
        free: true,
    });
    facts
        .plan_windows
        .push(plan(101, d(2024, 4, 1), 1, d(2024, 1, 15)));
    facts.fees.push(FeeFact {
        to: Some(d(2024, 4, 1)),
        ..fixed(10, 1, d(2024, 1, 1), 3_100)
    });
    facts.fees.push(fixed(20, 2, d(2024, 4, 1), 5_000));
    facts.ending = Some(Ending {
        at: at(2024, 5, 10, 0),
        kind: EndingKind::Cancel,
    });

    let mut daily = Controller::inserted(&facts, at(2023, 12, 20, 0));
    let mut transitions = Vec::new();
    let mut day = at(2023, 12, 21, 6);
    while day <= at(2024, 6, 1, 6) {
        transitions.extend(daily.apply(&facts, day).transitions);
        assert!(daily.step(&facts, day).is_empty(), "again at {day}");
        day += Duration::days(1);
    }

    let mut late = Controller::inserted(&facts, at(2023, 12, 20, 0));
    let once = late.apply(&facts, at(2024, 6, 1, 6));
    assert!(late.step(&facts, at(2024, 6, 1, 6)).is_empty());
    assert_eq!(
        once.transitions,
        vec![
            status(Status::Pending, Status::Trialing, at(2024, 1, 1, 0)),
            status(Status::Trialing, Status::TrialExpired, at(2024, 1, 15, 0)),
            status(Status::TrialExpired, Status::Active, at(2024, 2, 10, 12)),
            Transition::Plan {
                from: id(100),
                to: id(101),
                at: at(2024, 4, 1, 0),
            },
            status(Status::Active, Status::Cancelled, at(2024, 5, 10, 0)),
        ]
    );
    assert_eq!(transitions, once.transitions);
    assert_eq!(daily.stored, late.stored);
    let set = |slots: &[DocumentSlot]| slots.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(set(&daily.written), set(&late.written));
}

fn wake(
    facts: &Facts<TestFee>,
    now: DateTime<Utc>,
    deadlines: &Deadlines<'_>,
) -> Option<DateTime<Utc>> {
    let evaluation = run(facts, now);
    let stored = Stored::from(&evaluation.state);
    step(
        facts,
        now,
        &evaluation,
        &stored,
        &[recurring(d(2024, 1, 1))],
        deadlines,
    )
    .next_wake_at
}

/// The controller wakes at the earliest of the evaluation's next wake, an invoice's deadlines and
/// a credit lapse after its grace; a deadline already passed is due at once.
/// Rules: OQ15
#[test]
fn the_next_wake_is_the_earliest_deadline() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 3_100));
    let now = at(2024, 1, 10, 0);
    assert_eq!(wake(&facts, now, &no_deadlines()), Some(at(2024, 2, 1, 0)));

    let finalize = [at(2024, 1, 12, 0)];
    let attempt = [at(2024, 1, 11, 0)];
    let lapse = [at(2024, 1, 10, 20)];
    let finalizing = Deadlines {
        finalize_after: &finalize,
        ..no_deadlines()
    };
    assert_eq!(wake(&facts, now, &finalizing), Some(at(2024, 1, 12, 0)));
    let retrying = Deadlines {
        next_payment_attempt_at: &attempt,
        ..finalizing
    };
    assert_eq!(wake(&facts, now, &retrying), Some(at(2024, 1, 11, 0)));
    let lapsing = Deadlines {
        credit_lapses: &lapse,
        settlement_grace: Duration::hours(2),
        ..retrying
    };
    assert_eq!(wake(&facts, now, &lapsing), Some(at(2024, 1, 10, 22)));

    let passed = [at(2024, 1, 9, 0)];
    let overdue = Deadlines {
        finalize_after: &passed,
        ..no_deadlines()
    };
    assert_eq!(wake(&facts, now, &overdue), Some(at(2024, 1, 9, 0)));
}

/// Bills a fee priced 0 by its usage of one metric, the others as `TestPricer` does.
struct MeteredPricer;

impl Pricer<TestFee> for MeteredPricer {
    fn metrics(&self, fee: &TestFee) -> Vec<BillableMetricId> {
        if fee.price == 0 {
            vec![id(900)]
        } else {
            Vec::new()
        }
    }

    fn rates(&self, segment: &Segment<'_, TestFee>) -> Result<Vec<Rate>, PricingError> {
        Ok(TestPricer
            .rates(segment)?
            .into_iter()
            .filter(|rate| rate.amount != 0)
            .collect())
    }

    fn usage(
        &self,
        _: &Segment<'_, TestFee>,
        _: &UsageWindow,
        _: &UsageReads,
    ) -> Result<Vec<Rate>, PricingError> {
        Ok(Vec::new())
    }
}

fn metered_step(
    facts: &Facts<TestFee>,
    stored_at: DateTime<Utc>,
    now: DateTime<Utc>,
    written: &[DocumentSlot],
) -> Step {
    let stored = Stored::from(
        &evaluate(facts, stored_at, None, &MeteredPricer)
            .unwrap()
            .state,
    );
    let evaluation = evaluate(facts, now, None, &MeteredPricer).unwrap();
    step(facts, now, &evaluation, &stored, written, &no_deadlines())
}

/// Cancelled the day it started, a subscription billing usage alone owes that day's usage up to
/// the cancellation on its opening invoice, due with it though nothing is read yet.
/// Rules: R-016
#[test]
fn an_ending_on_the_start_day_opens_the_invoice_of_its_usage() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts
        .fees
        .push(fee(30, 3, d(2024, 1, 1), Billing::Arrears, 0));
    facts.ending = Some(Ending {
        at: at(2024, 1, 1, 14),
        kind: EndingKind::Cancel,
    });

    let ended = metered_step(&facts, at(2024, 1, 1, 1), at(2024, 1, 1, 15), &[]);
    assert_eq!(slots(&ended), vec![recurring(d(2024, 1, 1))]);
}

/// Cancelled on a renewal day after its invoice was written, the day's usage up to the
/// cancellation goes on the ending's document, due once its window closed.
/// Rules: R-016
#[test]
fn an_ending_after_its_day_was_invoiced_is_due_once_its_usage_window_closed() {
    let mut facts = subscription(d(2024, 1, 1), 1, 1);
    facts.fees.push(fixed(10, 1, d(2024, 1, 1), 3_100));
    facts
        .fees
        .push(fee(30, 3, d(2024, 1, 1), Billing::Arrears, 0));
    facts.ending = Some(Ending {
        at: at(2024, 2, 1, 14),
        kind: EndingKind::Cancel,
    });
    let written = [recurring(d(2024, 1, 1)), recurring(d(2024, 2, 1))];

    let open = metered_step(&facts, at(2024, 2, 1, 1), at(2024, 2, 1, 10), &written);
    assert!(!slots(&open).contains(&DocumentSlot::Final));
    let ended = metered_step(&facts, at(2024, 2, 1, 1), at(2024, 2, 1, 15), &written);
    assert!(slots(&ended).contains(&DocumentSlot::Final));
}
