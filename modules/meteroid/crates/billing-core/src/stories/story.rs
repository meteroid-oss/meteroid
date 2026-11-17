//! Generated stories of one subscription: a setup, then dated operations each written as facts at
//! the instant it is decided, so a run sees only what was known by then. Operations hold choices
//! only and resolve against the facts written before them, so a shrunk story stays valid.

use std::collections::BTreeMap;

use chrono::{DateTime, Days, Duration, NaiveDate, NaiveTime, Utc};
use common_domain::ids::{BillableMetricId, PlanVersionId};
use proptest::prelude::*;
use uuid::Uuid;

use crate::charge::Fee;
use crate::facts::{
    Billing, CouponFact, Ending, EndingKind, Facts, FeeFact, MinimumFact, MinimumScope, PlanWindow,
    SeatStep, Start, Trial,
};
use crate::timeline::Grid;

pub type At = DateTime<Utc>;

pub const METRIC: u128 = 9_000;

pub fn midnight(day: NaiveDate) -> At {
    day.and_time(NaiveTime::MIN).and_utc()
}

pub fn id<T: From<Uuid>>(n: u128) -> T {
    Uuid::from_u128(n).into()
}

pub fn metric() -> BillableMetricId {
    id(METRIC)
}

/// What a fee bills, as the story's pricer reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A fixed amount a period, prorated by days.
    Fixed,
    /// A fixed amount a period per seat, prorated by days.
    Seats,
    /// A fixed amount billed whole: prorated only where a change or a cancellation cuts it (R-002).
    Capacity,
    /// An amount per unit of the story's metric, in arrears, never prorated (R-002).
    Usage,
    /// A plan's pack: its fee and its grant prorated as a fixed fee's.
    Pack,
    /// A pack with its own duration, bought whole (R-075).
    BoughtPack,
    /// Billed once, on its day (R-075, R-093).
    OneTime,
}

impl Kind {
    pub fn prorated(self) -> bool {
        matches!(self, Kind::Fixed | Kind::Seats | Kind::Pack | Kind::Usage)
    }

    pub fn billing(self, arrears: bool) -> Billing {
        match self {
            Kind::Usage => Billing::Arrears,
            Kind::OneTime => Billing::OneTime,
            Kind::Fixed | Kind::Seats if arrears => Billing::Arrears,
            _ => Billing::Advance,
        }
    }

    /// The story's price for `kind` from a drawn number: per unit for usage, per seat for seats.
    pub fn amount(self, drawn: i64) -> i64 {
        match self {
            Kind::Usage => 1 + drawn % 100,
            Kind::Seats => drawn % 5_000,
            _ => drawn,
        }
    }
}

/// A fee row's price.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Price {
    pub kind: Kind,
    pub amount: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct FeeSpec {
    pub kind: Kind,
    pub arrears: bool,
    /// Its own cadence, else the plan's.
    pub cadence: Option<u32>,
    pub price: i64,
}

#[derive(Clone, Debug)]
pub struct Setup {
    /// Days after 2023-01-01.
    pub start: u16,
    pub anchor: u32,
    pub cadence: u32,
    /// Days, and whether it is free.
    pub trial: Option<(u8, bool)>,
    pub fees: Vec<FeeSpec>,
    pub seats: u8,
    pub minimum: Option<i64>,
    pub coupons: u8,
    /// The reconcile watermark, as the n-th period start of the plan's grid.
    pub watermark: Option<u8>,
    /// Whether take-backs go by unused time past the step's day rather than latest lines first.
    pub unused_first: bool,
}

/// `periods` renewals after the period of the operation before, then `day` days into it: 0 is
/// the renewal day; never before the operation before.
#[derive(Clone, Copy, Debug)]
pub struct When {
    pub periods: u8,
    pub day: u8,
}

#[derive(Clone, Copy, Debug)]
pub enum Decided {
    /// During its day, at this hour: an immediate change.
    Same(u8),
    /// That many days before its day: a scheduled change.
    Ahead(u8),
}

#[derive(Clone, Copy, Debug)]
pub enum How {
    /// At the instant it is decided (R-016).
    Immediate,
    /// At its day's midnight.
    Dated,
    /// At the end of the period holding its day.
    PeriodEnd,
}

#[derive(Clone, Copy, Debug)]
pub enum Op {
    /// A new row for a fee from its day: its lineage kept (an override) or a new one, the old row
    /// closed or left open under the new one.
    Amend {
        fee: u8,
        when: When,
        decided: Decided,
        price: i64,
        cadence: Option<u32>,
        new_lineage: bool,
        overlap: bool,
    },
    Seats {
        when: When,
        decided: Decided,
        quantity: u8,
    },
    AddOn {
        spec: FeeSpec,
        when: When,
        decided: Decided,
        life: Option<u16>,
    },
    Remove {
        fee: u8,
        when: When,
        decided: Decided,
    },
    /// A new plan window from its day: the plan's fees closed there and opened again, on their
    /// lineages or new ones, at `percent` of their price.
    PlanChange {
        when: When,
        decided: Decided,
        cadence: u32,
        keep_lineage: bool,
        percent: u8,
        setup_fee: Option<i64>,
    },
    Cancel {
        when: When,
        decided: Decided,
        how: How,
    },
    /// The document a step issued, voided during its day (R-096).
    Void { when: When, hour: u8, pick: u8 },
}

/// How the story ends when no operation cancelled it.
#[derive(Clone, Copy, Debug)]
pub enum End {
    Cancel {
        when: When,
        decided: Decided,
        how: How,
    },
    /// A fixed term ending on this day, known from the start (R-084).
    Term { when: When },
}

#[derive(Clone, Debug)]
pub struct Story {
    pub setup: Setup,
    pub ops: Vec<Op>,
    pub end: End,
}

// ---- strategies

fn kind() -> impl Strategy<Value = Kind> {
    prop_oneof![
        4 => Just(Kind::Fixed),
        3 => Just(Kind::Seats),
        2 => Just(Kind::Capacity),
        3 => Just(Kind::Usage),
        1 => Just(Kind::Pack),
        1 => Just(Kind::BoughtPack),
        1 => Just(Kind::OneTime),
    ]
}

fn cadence() -> impl Strategy<Value = u32> {
    prop::sample::select(vec![1u32, 1, 3, 12])
}

fn fee_spec() -> impl Strategy<Value = FeeSpec> {
    (
        kind(),
        any::<bool>(),
        prop::option::weighted(0.25, cadence()),
        0i64..40_000,
    )
        .prop_map(|(kind, arrears, cadence, price)| FeeSpec {
            kind,
            arrears,
            cadence,
            price,
        })
}

fn when() -> impl Strategy<Value = When> {
    (
        prop::sample::select(vec![0u8, 0, 1, 1, 2, 3]),
        prop_oneof![3 => Just(0u8), 1 => Just(1u8), 4 => 2u8..40],
    )
        .prop_map(|(periods, day)| When { periods, day })
}

fn decided() -> impl Strategy<Value = Decided> {
    prop_oneof![
        2 => (0u8..24).prop_map(Decided::Same),
        1 => (1u8..20).prop_map(Decided::Ahead),
    ]
}

fn how() -> impl Strategy<Value = How> {
    prop_oneof![Just(How::Immediate), Just(How::Dated), Just(How::PeriodEnd)]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (any::<u8>(), when(), decided(), 0i64..40_000,
              prop::option::weighted(0.3, cadence()), any::<bool>(), prop::bool::weighted(0.2))
            .prop_map(|(fee, when, decided, price, cadence, new_lineage, overlap)| Op::Amend {
                fee, when, decided, price, cadence, new_lineage, overlap,
            }),
        3 => (when(), decided(), 0u8..20)
            .prop_map(|(when, decided, quantity)| Op::Seats { when, decided, quantity }),
        2 => (fee_spec(), when(), decided(), prop::option::weighted(0.4, 1u16..200))
            .prop_map(|(spec, when, decided, life)| Op::AddOn { spec, when, decided, life }),
        1 => (any::<u8>(), when(), decided())
            .prop_map(|(fee, when, decided)| Op::Remove { fee, when, decided }),
        3 => (when(), decided(), cadence(), any::<bool>(), 50u8..=200,
              prop::option::weighted(0.3, 1i64..20_000))
            .prop_map(|(when, decided, cadence, keep_lineage, percent, setup_fee)| {
                Op::PlanChange { when, decided, cadence, keep_lineage, percent, setup_fee }
            }),
        1 => (when(), decided(), how())
            .prop_map(|(when, decided, how)| Op::Cancel { when, decided, how }),
        2 => (when(), 0u8..24, any::<u8>())
            .prop_map(|(when, hour, pick)| Op::Void { when, hour, pick }),
    ]
}

fn end() -> impl Strategy<Value = End> {
    prop_oneof![
        3 => (when(), decided(), how())
            .prop_map(|(when, decided, how)| End::Cancel { when, decided, how }),
        1 => when().prop_map(|when| End::Term { when }),
    ]
}

prop_compose! {
    fn setup()(
        start in 0u16..120,
        anchor in prop::sample::select(vec![1u32, 1, 15, 28, 29, 30, 31]),
        cadence in cadence(),
        trial in prop::option::weighted(0.25, (1u8..40, any::<bool>())),
        fees in prop::collection::vec(fee_spec(), 1..4),
        seats in 0u8..12,
        minimum in prop::option::weighted(0.2, 0i64..30_000),
        coupons in 0u8..3,
        watermark in prop::option::weighted(0.3, 0u8..8),
        unused_first in any::<bool>(),
    ) -> Setup {
        Setup { start, anchor, cadence, trial, fees, seats, minimum, coupons, watermark, unused_first }
    }
}

pub fn story() -> impl Strategy<Value = Story> {
    (setup(), prop::collection::vec(op(), 0..9), end()).prop_map(|(setup, ops, end)| Story {
        setup,
        ops,
        end,
    })
}

// ---- writing the facts

/// A story's facts as known at each instant, and what happens to its documents.
#[derive(Clone, Debug)]
pub struct Built {
    /// Each decision's instant and the facts known from it on, in order.
    pub snapshots: Vec<(At, Facts<Price>)>,
    /// Voids: when, and which document.
    pub voids: Vec<(At, u8)>,
    /// The watermark's day and the instant it is set, after that day's midnight was billed.
    pub watermark: Option<(NaiveDate, At)>,
    pub unused_first: bool,
    /// The last instant run: everything is due and issued by then.
    pub until: At,
    /// The story as read by a person: one line per operation that took effect.
    pub narrative: Vec<String>,
    pub names: BTreeMap<Fee, String>,
}

impl Built {
    /// The facts known at `at`.
    pub fn facts_at(&self, at: At) -> &Facts<Price> {
        let n = self
            .snapshots
            .partition_point(|(decided, _)| *decided <= at);
        &self.snapshots[n.saturating_sub(1)].1
    }

    pub fn last(&self) -> &Facts<Price> {
        &self.snapshots[self.snapshots.len() - 1].1
    }
}

/// The grid the plan in force on `at` renews on, as R-084 places it: from the billing start, or
/// from the origin of a change of cadence.
pub fn plan_grid<F>(facts: &Facts<F>, at: NaiveDate) -> Option<Grid> {
    let window = facts.plan_on(at)?;
    grid_for(facts, window.cadence_origin, window.cadence)
}

/// The grid of `months` from `origin`, or from the billing start when the origin is not after it.
pub fn grid_for<F>(facts: &Facts<F>, origin: NaiveDate, months: u32) -> Option<Grid> {
    let start = facts.billing_start();
    let anchor = facts.billing_day_anchor;
    if origin <= start {
        if facts.bills_from_trial_end() {
            Grid::from_trial_end(start, anchor, months.max(1))
        } else {
            Grid::from_start(start, anchor, months.max(1))
        }
    } else {
        Grid::from_origin(origin, anchor, months.max(1))
    }
}

struct Builder {
    facts: Facts<Price>,
    next: u128,
    names: BTreeMap<Fee, String>,
    kinds: BTreeMap<Fee, Kind>,
    prev_day: NaiveDate,
    prev_decided: At,
    last_day: NaiveDate,
    ending: Option<Ending>,
    snapshots: Vec<(At, Facts<Price>)>,
    voids: Vec<(At, u8)>,
    narrative: Vec<String>,
    /// A plan change moved the subscription onto a grid of its own: amendments open no window.
    own_grid: bool,
}

fn when_text(at: At) -> String {
    at.format("%Y-%m-%d %H:%M").to_string()
}

impl Builder {
    fn id(&mut self) -> u128 {
        self.next += 1;
        self.next
    }

    fn lineage(&mut self, component: bool, kind: Kind) -> Fee {
        let n = self.id();
        let fee = if component {
            Fee::Component(id(n))
        } else {
            Fee::AddOn(id(n))
        };
        let name = format!("{}{n} {kind:?}", if component { "c" } else { "a" });
        self.names.insert(fee.clone(), name);
        self.kinds.insert(fee.clone(), kind);
        fee
    }

    fn name(&self, fee: &Fee) -> String {
        self.names.get(fee).cloned().unwrap_or_default()
    }

    fn row(
        &mut self,
        lineage: &Fee,
        from: NaiveDate,
        billing: Billing,
        cadence: u32,
        price: Price,
        plan_component: bool,
    ) {
        let n = self.id();
        self.facts.fees.push(FeeFact {
            id: Uuid::from_u128(n),
            lineage: lineage.clone(),
            from,
            to: None,
            billing,
            cadence,
            plan_component,
            fee: price,
        });
    }

    fn close(&mut self, row: Uuid, on: NaiveDate) {
        if let Some(row) = self.facts.fees.iter_mut().find(|r| r.id == row) {
            row.to = Some(row.to.map_or(on, |to| to.min(on)));
        }
    }

    /// The lineages billing on `on`, one-time fees left out, in the order they were written.
    fn live(&self, on: NaiveDate, components_only: bool) -> Vec<Fee> {
        let mut live: Vec<Fee> = Vec::new();
        for row in &self.facts.fees {
            let fee = &row.lineage;
            if live.contains(fee)
                || (components_only && !matches!(fee, Fee::Component(_)))
                || row.billing == Billing::OneTime
            {
                continue;
            }
            if self.facts.fee_on(fee, on).is_some() {
                live.push(fee.clone());
            }
        }
        live
    }

    fn resolve(&self, when: When) -> NaiveDate {
        let Some(grid) = plan_grid(&self.facts, self.prev_day) else {
            return self.prev_day;
        };
        let Some(mut period) = grid.period_containing(self.prev_day) else {
            return self.prev_day;
        };
        for _ in 0..when.periods {
            match grid.period_containing(period.1) {
                Some(next) if next.0 > period.0 => period = next,
                _ => break,
            }
        }
        (period.0 + Days::new(u64::from(when.day))).max(self.prev_day)
    }

    /// When an operation dated `on` is decided, never before the one before it.
    fn decide(&self, on: NaiveDate, decided: Decided) -> At {
        let at = match decided {
            Decided::Same(hour) => midnight(on) + Duration::hours(i64::from(hour % 24)),
            Decided::Ahead(days) => midnight(on - Days::new(u64::from(days))) + Duration::hours(1),
        };
        at.max(self.prev_decided)
    }

    /// Whether an operation dated `on` and decided at `at` still applies: before the ending, within
    /// the story's span.
    fn applies(&self, on: NaiveDate, at: At) -> bool {
        on <= self.last_day
            && self
                .ending
                .is_none_or(|ending| on < ending.on() && at < ending.at)
    }

    fn record(&mut self, decided: At, on: NaiveDate, text: String) {
        self.prev_day = on;
        self.prev_decided = decided;
        self.snapshots.push((decided, self.facts.clone()));
        self.narrative
            .push(format!("{on} (decided {}): {text}", when_text(decided)));
    }

    /// R-084: an amendment leaving fees of another shortest cadence renews on it from `on`, on
    /// the grid it was created on, as the store opens the plan window; none in a trial, nor once a
    /// plan change gave the subscription a grid of its own.
    fn amendment_cadence(&mut self, on: NaiveDate, text: String) -> String {
        let in_trial = self.facts.trial.is_some_and(|trial| on < trial.until);
        let Some(current) = self.facts.plan_on(on).copied() else {
            return text;
        };
        let shortest = self
            .live(on, false)
            .iter()
            .filter_map(|fee| self.facts.fee_on(fee, on).map(|row| row.cadence.max(1)))
            .min();
        let Some(shortest) = shortest.filter(|c| *c != current.cadence) else {
            return text;
        };
        if self.own_grid || in_trial {
            return text;
        }
        if let Some(open) = self.facts.plan_windows.iter_mut().find(|w| w.to.is_none()) {
            if open.from == on {
                open.cadence = shortest;
                return format!("{text}; renews every {shortest} month(s)");
            }
            open.to = Some(on);
        }
        self.facts.plan_windows.push(PlanWindow {
            plan_version: current.plan_version,
            from: on,
            to: None,
            cadence: shortest,
            cadence_origin: current.cadence_origin,
        });
        format!("{text}; renews every {shortest} month(s)")
    }

    fn seats_of(&self, lineage: &Fee, on: NaiveDate) -> u64 {
        self.facts
            .seats_on(lineage, on)
            .map_or(1, |step| step.quantity)
    }

    fn seat_step(&mut self, lineage: &Fee, on: NaiveDate, quantity: u64) {
        let n = self.id();
        self.facts.seats.push(SeatStep {
            id: id(n),
            lineage: lineage.clone(),
            at: on,
            quantity,
        });
    }

    fn apply(&mut self, op: Op) {
        match op {
            Op::Amend {
                fee,
                when,
                decided,
                price,
                cadence,
                new_lineage,
                overlap,
            } => {
                let on = self.resolve(when);
                let at = self.decide(on, decided);
                let live = self.live(on, false);
                if live.is_empty() || !self.applies(on, at) {
                    return;
                }
                let lineage = live[usize::from(fee) % live.len()].clone();
                let Some(old) = self.facts.fee_on(&lineage, on).cloned() else {
                    return;
                };
                let kind = old.fee.kind;
                // A plan's fee follows its plan's cadence: only an add-on changes its own.
                let cadence = match (&lineage, cadence) {
                    (Fee::AddOn(_), Some(cadence)) if kind != Kind::OneTime => cadence,
                    _ => old.cadence,
                };
                let price = Price {
                    kind,
                    amount: kind.amount(price),
                };
                let target = if new_lineage {
                    let fresh = self.lineage(matches!(lineage, Fee::Component(_)), kind);
                    if kind == Kind::Seats {
                        let seats = self.seats_of(&lineage, on);
                        self.seat_step(&fresh, on, seats);
                    }
                    fresh
                } else {
                    lineage.clone()
                };
                if new_lineage || !overlap {
                    self.close(old.id, on);
                }
                // An override keeps its row's origin; a new lineage is the amendment's own.
                let plan_component = old.plan_component && !new_lineage;
                self.row(&target, on, old.billing, cadence, price, plan_component);
                if let Some(to) = old.to.filter(|_| !overlap || new_lineage) {
                    let row = self.facts.fees.last().map(|row| row.id);
                    if let Some(row) = row {
                        self.close(row, to);
                    }
                }
                let text = format!(
                    "amend {} to {} every {cadence} month(s){}{}",
                    self.name(&lineage),
                    price.amount,
                    if new_lineage {
                        format!(" as {}", self.name(&target))
                    } else {
                        " on its lineage".to_owned()
                    },
                    if overlap && !new_lineage {
                        ", the old row left open under it"
                    } else {
                        ""
                    }
                );
                let text = self.amendment_cadence(on, text);
                self.record(at, on, text);
            }
            Op::Seats {
                when,
                decided,
                quantity,
            } => {
                let on = self.resolve(when);
                let at = self.decide(on, decided);
                let lineage = self
                    .live(on, false)
                    .into_iter()
                    .find(|fee| self.kinds.get(fee) == Some(&Kind::Seats));
                let Some(lineage) = lineage else { return };
                if !self.applies(on, at) {
                    return;
                }
                self.seat_step(&lineage, on, u64::from(quantity));
                let text = format!("{} to {quantity} seat(s)", self.name(&lineage));
                self.record(at, on, text);
            }
            Op::AddOn {
                spec,
                when,
                decided,
                life,
            } => {
                let on = self.resolve(when);
                let at = self.decide(on, decided);
                if !self.applies(on, at) {
                    return;
                }
                let plan = self.facts.plan_on(on).map_or(1, |window| window.cadence);
                let lineage = self.lineage(false, spec.kind);
                let cadence = spec.cadence.unwrap_or(plan);
                let price = Price {
                    kind: spec.kind,
                    amount: spec.kind.amount(spec.price),
                };
                if spec.kind == Kind::Seats {
                    self.seat_step(&lineage, on, 1);
                }
                self.row(
                    &lineage,
                    on,
                    spec.kind.billing(spec.arrears),
                    cadence,
                    price,
                    false,
                );
                let to = life
                    .filter(|_| spec.kind != Kind::OneTime)
                    .map(|days| on + Days::new(u64::from(days)));
                if let Some(to) = to {
                    let row = self.facts.fees.last().map(|row| row.id);
                    if let Some(row) = row {
                        self.close(row, to);
                    }
                }
                let text = format!(
                    "add {} at {} every {cadence} month(s), {:?}{}",
                    self.name(&lineage),
                    price.amount,
                    spec.kind.billing(spec.arrears),
                    to.map(|to| format!(", until {to}")).unwrap_or_default()
                );
                let text = self.amendment_cadence(on, text);
                self.record(at, on, text);
            }
            Op::Remove { fee, when, decided } => {
                let on = self.resolve(when);
                let at = self.decide(on, decided);
                let live = self.live(on, false);
                if live.is_empty() || !self.applies(on, at) {
                    return;
                }
                let lineage = live[usize::from(fee) % live.len()].clone();
                let rows: Vec<Uuid> = self
                    .facts
                    .fees
                    .iter()
                    .filter(|row| row.lineage == lineage)
                    .map(|row| row.id)
                    .collect();
                for row in rows {
                    self.close(row, on);
                }
                let text = format!("remove {}", self.name(&lineage));
                let text = self.amendment_cadence(on, text);
                self.record(at, on, text);
            }
            Op::PlanChange {
                when,
                decided,
                cadence,
                keep_lineage,
                percent,
                setup_fee,
            } => self.plan_change(when, decided, cadence, keep_lineage, percent, setup_fee),
            Op::Cancel { when, decided, how } => self.cancel(when, decided, how),
            Op::Void { when, hour, pick } => {
                let on = self.resolve(when);
                let at =
                    (midnight(on) + Duration::hours(i64::from(hour % 24))).max(self.prev_decided);
                if on > self.last_day {
                    return;
                }
                self.voids.push((at, pick));
                self.prev_day = on;
                self.prev_decided = at;
                self.narrative.push(format!(
                    "{on} (at {}): void a document issued before (pick {pick})",
                    when_text(at)
                ));
            }
        }
    }

    fn plan_change(
        &mut self,
        when: When,
        decided: Decided,
        cadence: u32,
        keep_lineage: bool,
        percent: u8,
        setup_fee: Option<i64>,
    ) {
        let on = self.resolve(when);
        let at = self.decide(on, decided);
        if !self.applies(on, at) {
            return;
        }
        let Some(current) = self.facts.plan_on(on).copied() else {
            return;
        };
        // R-084: the change's first renewal is its origin: its day on a boundary of the grid in
        // force, else the new cadence's first boundary after it through the old grid's next one.
        let origin = if cadence == current.cadence {
            current.cadence_origin
        } else {
            let Some(grid) = plan_grid(&self.facts, on) else {
                return;
            };
            if grid.is_boundary(on) {
                on
            } else {
                let Some((_, next)) = grid.period_containing(on) else {
                    return;
                };
                let renewal = Grid::from_origin(next, self.facts.billing_day_anchor, cadence)
                    .and_then(|grid| grid.period_containing(on))
                    .map(|(_, renewal)| renewal);
                let Some(renewal) = renewal else { return };
                renewal
            }
        };
        if let Some(window) = self
            .facts
            .plan_windows
            .iter_mut()
            .find(|window| window.to.is_none())
        {
            window.to = Some(on);
        }
        self.own_grid |= cadence != current.cadence;
        let version = self.id();
        self.facts.plan_windows.push(PlanWindow {
            plan_version: id::<PlanVersionId>(version),
            from: on,
            to: None,
            cadence,
            cadence_origin: origin,
        });
        let mut changed: Vec<String> = Vec::new();
        for lineage in self.live(on, true) {
            let Some(old) = self.facts.fee_on(&lineage, on).cloned() else {
                continue;
            };
            let kind = old.fee.kind;
            let target = if keep_lineage {
                lineage.clone()
            } else {
                let fresh = self.lineage(true, kind);
                if kind == Kind::Seats {
                    let seats = self.seats_of(&lineage, on);
                    self.seat_step(&fresh, on, seats);
                }
                fresh
            };
            let rows: Vec<Uuid> = self
                .facts
                .fees
                .iter()
                .filter(|row| row.lineage == lineage)
                .map(|row| row.id)
                .collect();
            for row in rows {
                self.close(row, on);
            }
            let row_cadence = if old.cadence == current.cadence {
                cadence
            } else {
                old.cadence
            };
            let amount =
                (old.fee.amount * i64::from(percent) / 100).max(i64::from(kind == Kind::Usage));
            self.row(
                &target,
                on,
                old.billing,
                row_cadence,
                Price { kind, amount },
                true,
            );
            changed.push(format!(
                "{} -> {} at {amount} every {row_cadence} month(s)",
                self.name(&lineage),
                self.name(&target)
            ));
        }
        if let Some(amount) = setup_fee {
            let lineage = self.lineage(true, Kind::OneTime);
            self.row(
                &lineage,
                on,
                Billing::OneTime,
                cadence,
                Price {
                    kind: Kind::OneTime,
                    amount,
                },
                true,
            );
            changed.push(format!("setup fee {} at {amount}", self.name(&lineage)));
        }
        let text = format!(
            "plan change to every {cadence} month(s) from origin {origin}: {}",
            changed.join(", ")
        );
        self.record(at, on, text);
    }

    fn cancel(&mut self, when: When, decided: Decided, how: How) {
        let resolved = self.resolve(when);
        let mut decided_at = self.decide(resolved, decided);
        let (on, at) = match how {
            How::Immediate => {
                let at = match decided {
                    Decided::Same(_) => decided_at,
                    Decided::Ahead(_) => {
                        (midnight(resolved) + Duration::hours(12)).max(self.prev_decided)
                    }
                };
                decided_at = at;
                (at.date_naive(), at)
            }
            How::Dated => (resolved, midnight(resolved)),
            How::PeriodEnd => {
                let end = plan_grid(&self.facts, resolved)
                    .and_then(|grid| grid.period_containing(resolved))
                    .map_or(resolved, |(_, end)| end);
                (end, midnight(end))
            }
        };
        // A cancellation decided after its instant applies at the decision (R-016).
        let at = at.max(decided_at);
        let on = on.max(at.date_naive());
        if on > self.last_day || self.ending.is_some() {
            return;
        }
        let ending = Ending {
            at,
            kind: EndingKind::Cancel,
        };
        self.ending = Some(ending);
        self.facts.ending = Some(ending);
        let text = format!("cancel {how:?}, ending at {}", when_text(at));
        self.record(decided_at.min(at), resolved.min(on), text);
        self.prev_day = on.max(self.prev_day);
    }
}

pub fn build(story: &Story) -> Built {
    let setup = &story.setup;
    let start =
        NaiveDate::from_ymd_opt(2023, 1, 1).expect("a day") + Days::new(u64::from(setup.start));
    let mut builder = Builder {
        facts: Facts {
            subscription: id(1),
            billing_day_anchor: setup.anchor,
            start: Start {
                date: start,
                settled: Some(midnight(start)),
            },
            trial: setup.trial.map(|(days, free)| Trial {
                from: start,
                until: start + Days::new(u64::from(days)),
                free,
            }),
            plan_windows: vec![PlanWindow {
                plan_version: id(100),
                from: start,
                to: None,
                // A subscription renews on the shortest cadence of its fees, as at creation.
                cadence: setup
                    .fees
                    .iter()
                    .filter(|fee| fee.kind != Kind::OneTime)
                    .map(|fee| fee.cadence.unwrap_or(setup.cadence))
                    .min()
                    .unwrap_or(setup.cadence),
                cadence_origin: start,
            }],
            fees: Vec::new(),
            seats: Vec::new(),
            coupons: Vec::new(),
            minimums: Vec::new(),
            ending: None,
            end_date: None,
            reconcile_from: None,
        },
        next: 1_000,
        names: BTreeMap::new(),
        kinds: BTreeMap::new(),
        prev_day: start,
        prev_decided: midnight(start),
        own_grid: false,
        last_day: start + Days::new(1_000),
        ending: None,
        snapshots: Vec::new(),
        voids: Vec::new(),
        narrative: Vec::new(),
    };
    let mut opened: Vec<String> = Vec::new();
    for spec in &setup.fees {
        let lineage = builder.lineage(true, spec.kind);
        let cadence = spec.cadence.unwrap_or(setup.cadence);
        let price = Price {
            kind: spec.kind,
            amount: spec.kind.amount(spec.price),
        };
        builder.row(
            &lineage,
            start,
            spec.kind.billing(spec.arrears),
            cadence,
            price,
            true,
        );
        if spec.kind == Kind::Seats {
            builder.seat_step(&lineage, start, u64::from(setup.seats));
        }
        opened.push(format!(
            "{} at {} every {cadence} month(s), {:?}",
            builder.name(&lineage),
            price.amount,
            spec.kind.billing(spec.arrears)
        ));
    }
    if let Some(amount) = setup.minimum {
        builder.facts.minimums.push(MinimumFact {
            id: Uuid::from_u128(700),
            from: start,
            to: None,
            amount,
            scope: MinimumScope::AllComponents,
        });
    }
    for n in 0..setup.coupons {
        builder.facts.coupons.push(CouponFact {
            id: id(800 + u128::from(n)),
            from: start,
            invoices: (n % 2 == 0).then_some(2),
        });
    }
    builder
        .snapshots
        .push((midnight(start), builder.facts.clone()));
    builder.narrative.push(format!(
        "{start}: start, anchor {}, every {} month(s), trial {:?}, minimum {:?}, {} coupon(s); {}",
        setup.anchor,
        setup.cadence,
        setup.trial,
        setup.minimum,
        setup.coupons,
        opened.join("; ")
    ));
    for op in &story.ops {
        builder.apply(*op);
    }
    if builder.ending.is_none() {
        match story.end {
            End::Cancel { when, decided, how } => builder.cancel(when, decided, how),
            End::Term { when } => {
                let on = builder.resolve(when).max(builder.prev_day + Days::new(1));
                for (_, facts) in &mut builder.snapshots {
                    facts.end_date = Some(on);
                }
                builder.facts.end_date = Some(on);
                builder
                    .narrative
                    .push(format!("{on}: the fixed term ends (known from the start)"));
                builder.ending = builder.facts.effective_ending();
            }
        }
    }
    if builder.ending.is_none() {
        // A cancellation past the story's span: end it at the last day instead.
        let at = midnight(builder.last_day).max(builder.prev_decided);
        let ending = Ending {
            at,
            kind: EndingKind::Cancel,
        };
        builder.facts.ending = Some(ending);
        builder.ending = Some(ending);
        builder.record(
            at,
            at.date_naive(),
            "cancel at the story's last day".to_owned(),
        );
    }
    let ending = builder.ending.expect("an ending");
    let final_facts = builder.facts.clone();
    // R-094: the start of a period whose recurring invoice was issued: a renewal of the plan in
    // force that day, none past the ending.
    let watermark = setup.watermark.map(|k| {
        let start = final_facts.billing_start();
        let mut renewals = (0..)
            .map(|n| start + Days::new(n))
            .take_while(|day| *day < ending.on())
            .filter(|day| {
                // A renewal as the facts stood at its midnight, when its invoice was issued.
                let known = builder
                    .snapshots
                    .partition_point(|(decided, _)| *decided <= midnight(*day));
                let facts = &builder.snapshots[known.saturating_sub(1)].1;
                *day == start || plan_grid(facts, *day).is_some_and(|g| g.is_boundary(*day))
            });
        let first = renewals.next().unwrap_or(start);
        let day = renewals.take(usize::from(k)).last().unwrap_or(first);
        (day, midnight(day) + Duration::minutes(30))
    });
    if let Some((day, _)) = watermark {
        builder.narrative.push(format!(
            "{day}: the reconcile watermark is set, after that midnight is billed"
        ));
    }
    let last_void = builder.voids.iter().map(|(at, _)| *at).max();
    let until = [
        Some(ending.at),
        Some(builder.prev_decided),
        last_void,
        watermark.map(|(_, at)| at),
    ]
    .into_iter()
    .flatten()
    .max()
    .expect("an instant")
        + Duration::days(1);
    Built {
        snapshots: builder.snapshots,
        voids: builder.voids,
        watermark,
        unused_first: setup.unused_first,
        until,
        narrative: builder.narrative,
        names: builder.names,
    }
}
