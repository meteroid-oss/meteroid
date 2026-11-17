//! What a subscription owes at an instant, from its facts alone: every charge due by then, the
//! document it goes on, the state the facts put it in and when to look again (R-084, R-079,
//! R-016, R-011).

use std::collections::BTreeMap;

use chrono::{DateTime, Months, NaiveDate, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::charge::{ChargeKey, ChargeKind, Denomination, Fee};
use crate::facts::{
    Billing, DatedFact, Ending, EndingKind, FactRef, Facts, FeeFact, MinimumScope, midnight,
};
use crate::pricer::{
    CreditGrant, InForce, Pricer, Rate, Segment, UsageRead, UsageReads, UsageWindow,
};
use crate::pricing::PricingError;
use crate::reconcile::{Expected, Term};
use crate::timeline::{Grid, Window, windows};

/// The document a charge goes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DocumentSlot {
    /// The recurring invoice of the subscription period starting on `period_start` (R-051).
    Recurring { period_start: NaiveDate },
    /// The final invoice of an ending, after the period opened that day was invoiced (R-016).
    Final,
    /// An adjustment invoice dated `on`.
    Adjustment { on: NaiveDate },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Charge {
    pub key: ChargeKey,
    /// The days it bills, `[from, until)`.
    pub window: (NaiveDate, NaiveDate),
    /// The instants its usage was read over.
    pub usage: Option<UsageWindow>,
    pub quantity: Option<Decimal>,
    /// Minor units. A commitment's amount is what its period's spend is trued up against, not
    /// money owed (R-022).
    pub amount: i64,
    pub due_at: DateTime<Utc>,
    pub document: DocumentSlot,
    pub trace: Vec<FactRef>,
    /// What a pack's charge grants, its share taken as the amount's.
    pub credit_grant: Option<CreditGrant>,
    /// See [`Rate::once_per_product`].
    pub once_per_product: Option<Uuid>,
}

impl Charge {
    /// A minimum's commitment for its period: its shortfall is reconcile's, from issued spend.
    pub fn is_commitment(&self) -> bool {
        self.key.kind == ChargeKind::Shortfall
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// Before the start, or awaiting a checkout start.
    Pending,
    Trialing,
    /// A free trial ended with its checkout start not settled.
    TrialExpired,
    Active,
    Cancelled,
    Completed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DerivedState {
    pub status: Status,
    /// The subscription period holding the instant; open-ended once ended.
    pub current_period: Option<(NaiveDate, Option<NaiveDate>)>,
    pub plan_version: Option<Uuid>,
    pub cadence: Option<u32>,
    pub pending_checkout: bool,
    /// The first period billed: its opening invoice bills no arrears and never trues up a
    /// minimum (R-011, R-022).
    pub first_period: Option<(NaiveDate, NaiveDate)>,
}

/// A read a usage window needs: an ended one to be priced, an open one for a draft.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UsageNeed {
    pub fee: Fee,
    pub read: UsageRead,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluation {
    pub charges: Vec<Charge>,
    pub state: DerivedState,
    pub next_wake: Option<DateTime<Utc>>,
    pub usage_needed: Vec<UsageNeed>,
}

impl Evaluation {
    /// What each charge should have billed by now, as reconcile reads it: the money charges
    /// summed per key, commitments left to the minimum's true-up.
    pub fn expected(&self) -> Vec<Expected> {
        let mut expected: Vec<Expected> = Vec::new();
        for charge in self.charges.iter().filter(|charge| !charge.is_commitment()) {
            match expected.iter_mut().find(|e| e.key == charge.key) {
                Some(e) => e.amount += charge.amount,
                None => expected.push(Expected {
                    key: charge.key.clone(),
                    amount: charge.amount,
                }),
            }
        }
        expected
    }
}

/// Every charge due by `now`: fees on their own grids cut to their life, the trial and the ending
/// (R-084, R-016); an advance window bills its price, then each change's difference when it takes
/// effect, the terms tiling it (R-079); usage per window, never prorated (R-002). A price the
/// pricer cannot compute fails the whole evaluation.
pub fn evaluate<F>(
    facts: &Facts<F>,
    now: DateTime<Utc>,
    usage: Option<&UsageReads>,
    pricer: &impl Pricer<F>,
) -> Result<Evaluation, PricingError> {
    let run = Run::new(facts, now, usage, pricer);
    run.evaluate()
}

/// A plan window's part of time, the grid its fees renew on.
#[derive(Clone, Copy, Debug)]
struct PlanSpan {
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    origin: NaiveDate,
    cadence: u32,
    plan: Option<FactRef>,
}

/// A fee's or minimum's window on its grid, with the period it is priced over.
#[derive(Clone, Copy, Debug)]
struct Billed {
    window: Window,
    nominal: (NaiveDate, NaiveDate),
    plan: Option<FactRef>,
}

/// A row of one lineage billing over `[from, to)`, its precedence over the others decided.
struct Interval<'a, F> {
    row: &'a FeeFact<F>,
    from: NaiveDate,
    to: Option<NaiveDate>,
}

struct UsagePiece<'a, F> {
    row: &'a FeeFact<F>,
    from: NaiveDate,
    until: NaiveDate,
    read: UsageWindow,
    on_grid: bool,
    cut: bool,
    plan: Option<FactRef>,
}

struct Run<'a, F, P> {
    facts: &'a Facts<F>,
    pricer: &'a P,
    usage: Option<&'a UsageReads>,
    now: DateTime<Utc>,
    today: NaiveDate,
    billing_start: Option<NaiveDate>,
    ending: Option<Ending>,
    end: Option<NaiveDate>,
    horizon: NaiveDate,
    cuts: Vec<NaiveDate>,
    spans: Vec<PlanSpan>,
    boundaries: Vec<NaiveDate>,
    lineages: Vec<Fee>,
}

#[derive(Default)]
struct Out {
    charges: Vec<Charge>,
    wakes: Vec<DateTime<Utc>>,
    needed: Vec<UsageNeed>,
}

impl<'a, F, P: Pricer<F>> Run<'a, F, P> {
    fn new(
        facts: &'a Facts<F>,
        now: DateTime<Utc>,
        usage: Option<&'a UsageReads>,
        pricer: &'a P,
    ) -> Self {
        let today = now.date_naive();
        let billing_start = facts.billing_start();
        let ending = facts.effective_ending();
        let end = ending.map(|ending| ending.on());
        let longest = facts
            .fees
            .iter()
            .map(|fee| fee.cadence)
            .chain(facts.plan_windows.iter().map(|window| window.cadence))
            .max()
            .unwrap_or(1)
            .max(1);
        let horizon = today
            .max(billing_start.unwrap_or(today))
            .checked_add_months(Months::new(2 * longest + 2))
            .unwrap_or(NaiveDate::MAX);

        let mut cuts: Vec<NaiveDate> = facts
            .dated()
            .flat_map(|fact| fact.cuts())
            .chain(billing_start)
            .chain(end)
            .collect();
        cuts.sort();
        cuts.dedup();

        let mut plans: Vec<_> = facts.plan_windows.iter().collect();
        plans.sort_by_key(|window| (window.from, window.plan_version));
        let mut spans: Vec<PlanSpan> = plans
            .iter()
            .enumerate()
            .map(|(i, window)| PlanSpan {
                from: (i > 0).then_some(window.from),
                to: plans.get(i + 1).map(|next| next.from),
                origin: window.cadence_origin,
                cadence: window.cadence.max(1),
                plan: Some(window.reference()),
            })
            .collect();
        if spans.is_empty() {
            spans.push(PlanSpan {
                from: None,
                to: None,
                origin: billing_start.unwrap_or(facts.start.date),
                cadence: 1,
                plan: None,
            });
        }

        let mut lineages: Vec<Fee> = facts.fees.iter().map(|fee| fee.lineage.clone()).collect();
        lineages.sort();
        lineages.dedup();

        let mut run = Run {
            facts,
            pricer,
            usage,
            now,
            today,
            billing_start,
            ending,
            end,
            horizon,
            cuts,
            spans,
            boundaries: Vec::new(),
            lineages,
        };
        run.boundaries = run.subscription_boundaries();
        run
    }

    fn evaluate(&self) -> Result<Evaluation, PricingError> {
        let mut out = Out::default();
        let mut prices: BTreeMap<usize, Vec<Rate>> = BTreeMap::new();
        let mut usage_pieces: Vec<UsagePiece<'_, F>> = Vec::new();
        if self.billing_start.is_some() {
            for lineage in &self.lineages {
                self.lineage(lineage, &mut prices, &mut usage_pieces, &mut out)?;
            }
            self.price_usage(usage_pieces, &mut out)?;
            let mut scopes: Vec<MinimumScope> =
                self.facts.minimums.iter().map(|m| m.scope).collect();
            scopes.sort();
            scopes.dedup();
            for scope in scopes {
                self.commitments(scope, &mut out);
            }
        }
        let state = self.state();
        let next_wake = self.next_wake(&out.wakes);
        let Out {
            mut charges,
            mut needed,
            ..
        } = out;
        charges.sort_by(|a, b| {
            (a.due_at, &a.key, a.window, a.usage, a.amount)
                .cmp(&(b.due_at, &b.key, b.window, b.usage, b.amount))
        });
        needed.sort();
        needed.dedup();
        Ok(Evaluation {
            charges,
            state,
            next_wake,
            usage_needed: needed,
        })
    }

    fn grid(&self, span: &PlanSpan, months: u32) -> Option<Grid> {
        let start = self.billing_start?;
        let anchor = self.facts.billing_day_anchor;
        if span.origin <= start {
            if self.facts.bills_from_trial_end() {
                Grid::from_trial_end(start, anchor, months)
            } else {
                Grid::from_start(start, anchor, months)
            }
        } else {
            Grid::from_origin(span.origin, anchor, months)
        }
    }

    fn span_on(&self, at: NaiveDate) -> &PlanSpan {
        self.spans
            .iter()
            .rev()
            .find(|span| span.from.is_none_or(|from| from <= at))
            .unwrap_or(&self.spans[0])
    }

    /// Where the subscription's periods start: the billing start, then each plan window's
    /// boundaries while it is in force, up to the ending.
    fn subscription_boundaries(&self) -> Vec<NaiveDate> {
        let Some(start) = self.billing_start else {
            return Vec::new();
        };
        let last = self.end.map_or(self.horizon, |end| end.min(self.horizon));
        let mut boundaries = vec![start];
        for span in &self.spans {
            let from = span.from.map_or(start, |from| from.max(start));
            let to = span.to.map_or(last, |to| to.min(last));
            let Some(grid) = self.grid(span, span.cadence) else {
                continue;
            };
            if from > to {
                continue;
            }
            if grid.is_boundary(from) {
                boundaries.push(from);
            }
            let mut cursor = from;
            while let Some((_, end)) = grid.period_containing(cursor) {
                if end > to || (end == to && span.to == Some(to)) {
                    break;
                }
                boundaries.push(end);
                cursor = end;
            }
        }
        boundaries.sort();
        boundaries.dedup();
        boundaries
    }

    fn is_boundary(&self, at: NaiveDate) -> bool {
        self.boundaries.binary_search(&at).is_ok()
    }

    fn ended_by_now(&self) -> bool {
        self.ending.is_some_and(|ending| ending.at <= self.now)
    }

    /// The document of the last charges of an ending: a final invoice of its own when the period
    /// opened that day was invoiced, else the recurring invoice opened on its date (R-016).
    fn ending_slot(&self) -> DocumentSlot {
        let (Some(ending), Some(end)) = (self.ending, self.end) else {
            return DocumentSlot::Adjustment { on: self.today };
        };
        if self.is_boundary(end) && ending.at > midnight(end) {
            DocumentSlot::Final
        } else {
            DocumentSlot::Recurring { period_start: end }
        }
    }

    /// A charge due on a boundary goes on that period's recurring invoice; one due on its fee's
    /// own grid between two boundaries rides the next renewal's, as the store bills it; one due
    /// at a change mid-period on an adjustment invoice.
    fn document(&self, due: DateTime<Utc>, on_grid: bool, cut: bool) -> DocumentSlot {
        if cut {
            return self.ending_slot();
        }
        let on = due.date_naive();
        if self.is_boundary(on) {
            return DocumentSlot::Recurring { period_start: on };
        }
        if let Some(end) = self.end
            && on >= end
        {
            return DocumentSlot::Recurring { period_start: end };
        }
        if on_grid {
            let next = self.boundaries.partition_point(|b| *b <= on);
            if let Some(next) = self.boundaries.get(next) {
                return DocumentSlot::Recurring {
                    period_start: *next,
                };
            }
            if self.end.is_some() {
                return self.ending_slot();
            }
        }
        DocumentSlot::Adjustment { on }
    }

    /// The rows of `lineage` in turn, from the billing start to the ending: on each day, the one
    /// with precedence.
    fn intervals(&self, lineage: &Fee) -> Vec<Interval<'a, F>> {
        let (Some(start), facts) = (self.billing_start, self.facts) else {
            return Vec::new();
        };
        let mut points: Vec<NaiveDate> = facts
            .fees
            .iter()
            .filter(|row| row.lineage == *lineage)
            .flat_map(|row| std::iter::once(row.from).chain(row.to))
            .chain(std::iter::once(start))
            .chain(self.end)
            .filter(|at| *at >= start && self.end.is_none_or(|end| *at <= end))
            .collect();
        points.sort();
        points.dedup();
        let mut intervals: Vec<Interval<'a, F>> = Vec::new();
        for (i, from) in points.iter().enumerate() {
            let to = points.get(i + 1).copied();
            if self.end.is_some_and(|end| *from >= end) {
                break;
            }
            let Some(row) = facts.fee_on(lineage, *from) else {
                continue;
            };
            match intervals.last_mut() {
                Some(last) if std::ptr::eq(last.row, row) && last.to == Some(*from) => {
                    last.to = to;
                }
                _ => intervals.push(Interval {
                    row,
                    from: *from,
                    to,
                }),
            }
        }
        intervals
    }

    /// The windows a run of rows billing alike lives through, on each plan window's grid in turn.
    /// What a new plan window's grid puts inside the period the run is billing stays in that
    /// period, so a lineage's charges never nest; its own periods start at the grid's next one.
    fn billed(
        &self,
        (from, to): (NaiveDate, Option<NaiveDate>),
        months: Option<u32>,
        before: Option<Billed>,
    ) -> Vec<Billed> {
        let mut billed: Vec<Billed> = Vec::new();
        for span in &self.spans {
            let a = span.from.map_or(from, |s| s.max(from));
            let b = match (span.to, to) {
                (Some(x), Some(y)) => Some(x.min(y)),
                (x, y) => x.or(y),
            };
            if b.is_some_and(|b| b <= a) {
                continue;
            }
            let Some(grid) = self.grid(span, months.unwrap_or(span.cadence)) else {
                continue;
            };
            for window in windows(&grid, (a, b), self.horizon) {
                let truncated = window.until == self.horizon
                    && window.until < window.period.1
                    && b.is_none_or(|b| b > self.horizon);
                if truncated {
                    continue;
                }
                let current = billed.last().copied().or(before);
                let inside = current.filter(|last| {
                    let (start, end) = last.window.period;
                    start <= window.from && window.until <= end
                });
                match (inside, billed.last_mut()) {
                    (Some(_), Some(last)) if last.window.until == window.from => {
                        last.window.until = window.until;
                    }
                    (Some(last), _) => billed.push(Billed {
                        window: Window {
                            period: last.window.period,
                            ..window
                        },
                        nominal: last.nominal,
                        plan: span.plan,
                    }),
                    (None, _) => billed.push(Billed {
                        window,
                        nominal: grid.nominal(window.period),
                        plan: span.plan,
                    }),
                }
            }
        }
        billed
    }

    /// Whether `period` ends by the reconcile watermark: its charges are settled for good.
    fn behind_watermark(&self, period: (NaiveDate, NaiveDate)) -> bool {
        self.facts
            .reconcile_from
            .is_some_and(|watermark| period.1 <= watermark)
    }

    /// Whether the ending cut `window` short of its period.
    fn cut_by_ending(&self, window: &Window) -> bool {
        self.end == Some(window.until) && window.until < window.period.1
    }

    fn lineage(
        &self,
        lineage: &Fee,
        prices: &mut BTreeMap<usize, Vec<Rate>>,
        usage_pieces: &mut Vec<UsagePiece<'a, F>>,
        out: &mut Out,
    ) -> Result<(), PricingError> {
        let intervals = self.intervals(lineage);
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for (i, interval) in intervals.iter().enumerate() {
            match runs.last_mut() {
                Some((_, end))
                    if intervals[*end].to == Some(interval.from)
                        && intervals[*end].row.billing == interval.row.billing
                        && intervals[*end].row.cadence == interval.row.cadence =>
                {
                    *end = i;
                }
                _ => runs.push((i, i)),
            }
        }
        let mut one_time_billed = false;
        let mut previous: Option<Billed> = None;
        for (first, last) in runs {
            let rows = &intervals[first..=last];
            let billing = rows[0].row.billing;
            let life = (rows[0].from, rows[rows.len() - 1].to);
            let months = (billing != Billing::OneTime).then_some(rows[0].row.cadence.max(1));
            let billed = self.billed(life, months, previous);
            previous = billed.last().copied().or(previous);
            if billing == Billing::OneTime {
                if !one_time_billed && let Some(first) = billed.first() {
                    self.one_time(lineage, first, prices, out)?;
                }
                one_time_billed = true;
                continue;
            }
            for window in &billed {
                self.fixed(lineage, billing, window, prices, out)?;
                self.usage_window(rows, window, usage_pieces, out);
            }
        }
        self.final_usage(lineage, usage_pieces, out);
        Ok(())
    }

    /// The rates of `lineage` from `from` on, priced with every fee billing then.
    fn rates_on(
        &self,
        lineage: &Fee,
        from: NaiveDate,
        prices: &mut BTreeMap<usize, Vec<Rate>>,
    ) -> Result<Vec<Rate>, PricingError> {
        let index = self
            .cuts
            .partition_point(|cut| *cut <= from)
            .saturating_sub(1);
        let rates = match prices.entry(index) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                let at = self
                    .cuts
                    .get(index)
                    .copied()
                    .filter(|cut| *cut <= from)
                    .unwrap_or(from);
                let segment = self.segment(at, self.cuts.get(index + 1).copied());
                entry.insert(self.pricer.rates(&segment)?)
            }
        };
        Ok(rates
            .iter()
            .filter(|rate| rate.fee == *lineage)
            .cloned()
            .collect())
    }

    /// The fees billing from `at` until `until`, each with its seats.
    fn segment(&self, at: NaiveDate, until: Option<NaiveDate>) -> Segment<'a, F> {
        let facts = self.facts;
        let live = self.billing_start.is_some_and(|start| at >= start)
            && self.end.is_none_or(|end| at < end);
        let fees = if live {
            self.lineages
                .iter()
                .filter_map(|lineage| {
                    let row = facts.fee_on(lineage, at)?;
                    Some(self.in_force(row, at))
                })
                .collect()
        } else {
            Vec::new()
        };
        Segment {
            from: at,
            until,
            fees,
        }
    }

    fn in_force(&self, row: &'a FeeFact<F>, at: NaiveDate) -> InForce<'a, F> {
        InForce {
            lineage: &row.lineage,
            row: row.id,
            billing: row.billing,
            cadence: row.cadence,
            seats: self
                .facts
                .seats_on(&row.lineage, at)
                .map(|step| step.quantity),
            fee: &row.fee,
        }
    }

    /// `[from, until)` split at every fact's cuts inside it.
    fn pieces(&self, from: NaiveDate, until: NaiveDate) -> Vec<(NaiveDate, NaiveDate)> {
        let inside = self
            .cuts
            .iter()
            .copied()
            .filter(|cut| from < *cut && *cut < until);
        let points: Vec<NaiveDate> = std::iter::once(from)
            .chain(inside)
            .chain(std::iter::once(until))
            .collect();
        points.windows(2).map(|pair| (pair[0], pair[1])).collect()
    }

    fn trace(&self, lineage: &Fee, at: NaiveDate, plan: Option<FactRef>) -> Vec<FactRef> {
        let facts = self.facts;
        let mut trace: Vec<FactRef> = facts
            .fee_on(lineage, at)
            .map(|row| row.reference())
            .into_iter()
            .chain(facts.seats_on(lineage, at).map(|step| step.reference()))
            .chain(plan)
            .chain(
                facts
                    .free_trial()
                    .filter(|trial| self.billing_start == Some(trial.until))
                    .map(|trial| trial.reference()),
            )
            .collect();
        trace.sort();
        trace.dedup();
        trace
    }

    fn with_ending(&self, mut trace: Vec<FactRef>, cut: bool) -> Vec<FactRef> {
        if cut && let Some(ending) = self.facts.ending_ref() {
            trace.push(ending);
            trace.sort();
            trace.dedup();
        }
        trace
    }

    fn due_at_end(&self, window: &Window) -> (DateTime<Utc>, bool) {
        let cut = self.cut_by_ending(window);
        match (cut, self.ending) {
            (true, Some(ending)) => (ending.at, true),
            _ => (midnight(window.until), false),
        }
    }

    /// The fixed charges of `lineage` over `billed`: in advance its price at the window's start,
    /// then each change's difference when it takes effect; in arrears the terms' sum at its end.
    /// A pack's grant follows its amount.
    fn fixed(
        &self,
        lineage: &Fee,
        billing: Billing,
        billed: &Billed,
        prices: &mut BTreeMap<usize, Vec<Rate>>,
        out: &mut Out,
    ) -> Result<(), PricingError> {
        let window = billed.window;
        if window.from >= window.until || self.behind_watermark(window.period) {
            return Ok(());
        }
        let (due, cut) = match billing {
            Billing::Arrears => self.due_at_end(&window),
            _ => (midnight(window.from), false),
        };
        if due > self.now {
            out.wakes.push(due);
            return Ok(());
        }
        let mut pieces: Vec<(NaiveDate, NaiveDate, Vec<Rate>)> = Vec::new();
        for (from, until) in self.pieces(window.from, window.until) {
            pieces.push((from, until, self.rates_on(lineage, from, prices)?));
        }
        let mut kinds: Vec<(ChargeKind, Denomination)> = pieces
            .iter()
            .flat_map(|(_, _, rates)| {
                rates
                    .iter()
                    .map(|rate| (rate.kind.clone(), rate.denomination))
            })
            .collect();
        kinds.sort();
        kinds.dedup();
        let key = |(kind, denomination): (ChargeKind, Denomination)| ChargeKey {
            subscription: Some(self.facts.subscription),
            fee: lineage.clone(),
            from: window.period.0,
            until: window.period.1,
            kind,
            denomination,
        };
        let share = |full: i64, prorated: bool, from: NaiveDate, until: NaiveDate| -> i64 {
            if prorated {
                Term {
                    fee: lineage.clone(),
                    full,
                    period: billed.nominal,
                    from,
                    until,
                }
                .amount()
            } else {
                full
            }
        };
        let part = |rate: Option<&Rate>, from: NaiveDate, until: NaiveDate| -> i64 {
            rate.map_or(0, |rate| share(rate.amount, rate.prorated, from, until))
        };
        let granted = |rate: Option<&Rate>, from: NaiveDate, until: NaiveDate| -> i64 {
            rate.and_then(|rate| rate.credit_grant)
                .map_or(0, |grant| share(grant.amount, grant.prorated, from, until))
        };
        let grant = |rate: Option<&Rate>, amount: i64| {
            rate.and_then(|rate| rate.credit_grant)
                .map(|grant| CreditGrant { amount, ..grant })
        };
        for charge in kinds {
            let mut terms: Vec<(NaiveDate, NaiveDate, Option<&Rate>)> = Vec::new();
            for (from, until, rates) in &pieces {
                let rate = rates
                    .iter()
                    .find(|rate| rate.kind == charge.0 && rate.denomination == charge.1);
                match terms.last_mut() {
                    Some(last) if last.2 == rate => last.1 = *until,
                    _ => terms.push((*from, *until, rate)),
                }
            }
            match billing {
                Billing::Arrears => {
                    let last = terms.iter().rev().find_map(|t| t.2);
                    let amount = if terms.iter().all(|t| t.2.is_none_or(|r| r.prorated)) {
                        terms.iter().map(|(a, b, rate)| part(*rate, *a, *b)).sum()
                    } else {
                        last.map_or(0, |r| r.amount)
                    };
                    let grants_prorated = terms.iter().all(|t| {
                        t.2.and_then(|r| r.credit_grant)
                            .is_none_or(|grant| grant.prorated)
                    });
                    let grant_amount = if grants_prorated {
                        terms
                            .iter()
                            .map(|(a, b, rate)| granted(*rate, *a, *b))
                            .sum()
                    } else {
                        granted(last, window.from, window.until)
                    };
                    let mut trace: Vec<FactRef> = terms
                        .iter()
                        .flat_map(|(from, _, _)| self.trace(lineage, *from, billed.plan))
                        .collect();
                    trace.sort();
                    trace.dedup();
                    out.charges.push(Charge {
                        key: key(charge),
                        window: (window.from, window.until),
                        usage: None,
                        quantity: last.and_then(|r| r.quantity),
                        amount,
                        due_at: due,
                        document: self.document(due, window.until == window.period.1, cut),
                        trace: self.with_ending(trace, cut),
                        credit_grant: grant(last, grant_amount),
                        once_per_product: last.and_then(|r| r.once_per_product),
                    });
                }
                _ => {
                    for (i, (from, _, rate)) in terms.iter().enumerate() {
                        let previous = i.checked_sub(1).and_then(|p| terms[p].2);
                        let amount =
                            part(*rate, *from, window.until) - part(previous, *from, window.until);
                        let grant_amount = granted(*rate, *from, window.until)
                            - granted(previous, *from, window.until);
                        if (i == 0 && rate.is_none()) || (i > 0 && amount == 0 && grant_amount == 0)
                        {
                            continue;
                        }
                        let due = midnight(*from);
                        if due > self.now {
                            out.wakes.push(due);
                            continue;
                        }
                        let on_grid = i == 0 && window.from == window.period.0;
                        let priced = rate.or(previous);
                        out.charges.push(Charge {
                            key: key(charge.clone()),
                            window: (*from, window.until),
                            usage: None,
                            quantity: priced.and_then(|r| r.quantity),
                            amount,
                            due_at: due,
                            document: self.document(due, on_grid, false),
                            trace: self.trace(lineage, *from, billed.plan),
                            credit_grant: grant(priced, grant_amount),
                            once_per_product: priced.and_then(|r| r.once_per_product),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// A one-time fee bills once, at the start of its life.
    fn one_time(
        &self,
        lineage: &Fee,
        billed: &Billed,
        prices: &mut BTreeMap<usize, Vec<Rate>>,
        out: &mut Out,
    ) -> Result<(), PricingError> {
        let window = billed.window;
        if self.behind_watermark(window.period) {
            return Ok(());
        }
        let due = midnight(window.from);
        if due > self.now {
            out.wakes.push(due);
            return Ok(());
        }
        let share = |full: i64, prorated: bool| {
            if prorated {
                Term {
                    fee: lineage.clone(),
                    full,
                    period: billed.nominal,
                    from: window.from,
                    until: window.until,
                }
                .amount()
            } else {
                full
            }
        };
        for rate in self.rates_on(lineage, window.from, prices)? {
            out.charges.push(Charge {
                key: ChargeKey {
                    subscription: Some(self.facts.subscription),
                    fee: lineage.clone(),
                    from: window.period.0,
                    until: window.period.1,
                    kind: rate.kind,
                    denomination: rate.denomination,
                },
                window: (window.from, window.until),
                usage: None,
                quantity: rate.quantity,
                amount: share(rate.amount, rate.prorated),
                due_at: due,
                document: self.document(due, window.from == window.period.0, false),
                trace: self.trace(lineage, window.from, billed.plan),
                credit_grant: rate.credit_grant.map(|grant| CreditGrant {
                    amount: share(grant.amount, grant.prorated),
                    ..grant
                }),
                once_per_product: rate.once_per_product,
            });
        }
        Ok(())
    }

    fn needs(&self, row: &FeeFact<F>, read: UsageWindow) -> Vec<UsageRead> {
        self.pricer
            .metrics(&row.fee)
            .into_iter()
            .map(|metric| UsageRead {
                metric,
                window: read,
            })
            .collect()
    }

    /// Queues `piece` for pricing when its window ended by now and its reads are there; lists the
    /// reads it needs otherwise.
    fn queue(
        &self,
        piece: UsagePiece<'a, F>,
        usage_pieces: &mut Vec<UsagePiece<'a, F>>,
        out: &mut Out,
    ) {
        let reads = self.needs(piece.row, piece.read);
        if reads.is_empty() {
            return;
        }
        let need = |read: UsageRead| UsageNeed {
            fee: piece.row.lineage.clone(),
            read,
        };
        if piece.read.until > self.now {
            out.wakes.push(piece.read.until);
            if piece.read.from <= self.now {
                out.needed.extend(reads.into_iter().map(need));
            }
            return;
        }
        let missing: Vec<UsageRead> = reads
            .into_iter()
            .filter(|read| self.usage.is_none_or(|usage| !usage.contains(read)))
            .collect();
        if missing.is_empty() {
            usage_pieces.push(piece);
        } else {
            out.needed.extend(missing.into_iter().map(need));
        }
    }

    /// The usage windows of `billed`: one per row billing in it, ending at the ending's instant
    /// when it cut the window (R-016).
    fn usage_window(
        &self,
        rows: &[Interval<'a, F>],
        billed: &Billed,
        usage_pieces: &mut Vec<UsagePiece<'a, F>>,
        out: &mut Out,
    ) {
        let window = billed.window;
        if self.behind_watermark(window.period) {
            return;
        }
        let cut = self.cut_by_ending(&window);
        for interval in rows {
            let from = interval.from.max(window.from);
            let until = interval.to.map_or(window.until, |to| to.min(window.until));
            if from >= until {
                continue;
            }
            let read_until = match (cut && until == window.until, self.ending) {
                (true, Some(ending)) => ending.at,
                _ => midnight(until),
            };
            self.queue(
                UsagePiece {
                    row: interval.row,
                    from,
                    until,
                    read: UsageWindow {
                        from: midnight(from),
                        until: read_until,
                    },
                    on_grid: until == window.period.1,
                    cut: cut && until == window.until,
                    plan: billed.plan,
                },
                usage_pieces,
                out,
            );
        }
    }

    /// What `lineage` consumed on the ending's day up to its instant, when no window of a period
    /// cut by the ending reads it: the ending falls on a boundary, or the fee started that day.
    fn final_usage(&self, lineage: &Fee, usage_pieces: &mut Vec<UsagePiece<'a, F>>, out: &mut Out) {
        let (Some(ending), Some(end), Some(start)) = (self.ending, self.end, self.billing_start)
        else {
            return;
        };
        if ending.at <= midnight(end) || end < start {
            return;
        }
        let Some(row) = self.facts.fee_on(lineage, end) else {
            return;
        };
        if row.billing == Billing::OneTime {
            return;
        }
        let span = self.span_on(end);
        let Some(grid) = self.grid(span, row.cadence.max(1)) else {
            return;
        };
        if grid
            .period_containing(end)
            .is_none_or(|period| period.0 != end || self.behind_watermark(period))
        {
            return;
        }
        self.queue(
            UsagePiece {
                row,
                from: end,
                until: end,
                read: UsageWindow {
                    from: midnight(end),
                    until: ending.at,
                },
                on_grid: false,
                cut: true,
                plan: span.plan,
            },
            usage_pieces,
            out,
        );
    }

    /// Prices the queued usage windows, those read over the same instants together.
    fn price_usage(
        &self,
        mut pieces: Vec<UsagePiece<'a, F>>,
        out: &mut Out,
    ) -> Result<(), PricingError> {
        let Some(reads) = self.usage else {
            return Ok(());
        };
        pieces.sort_by_key(|piece| (piece.read, piece.row.lineage.clone()));
        for group in pieces.chunk_by(|a, b| a.read == b.read) {
            let window = group[0].read;
            let segment = Segment {
                from: group[0].from,
                until: Some(group[0].until),
                fees: group
                    .iter()
                    .map(|piece| self.in_force(piece.row, piece.from))
                    .collect(),
            };
            for rate in self.pricer.usage(&segment, &window, reads)? {
                let Some(piece) = group.iter().find(|piece| piece.row.lineage == rate.fee) else {
                    continue;
                };
                let lineage = &piece.row.lineage;
                let trace = self.trace(lineage, piece.from, piece.plan);
                out.charges.push(Charge {
                    key: ChargeKey {
                        subscription: Some(self.facts.subscription),
                        fee: lineage.clone(),
                        from: piece.from,
                        until: piece.until,
                        kind: rate.kind,
                        denomination: rate.denomination,
                    },
                    window: (piece.from, piece.until),
                    usage: Some(window),
                    quantity: rate.quantity,
                    amount: rate.amount,
                    due_at: window.until,
                    document: self.document(window.until, piece.on_grid, piece.cut),
                    trace: self.with_ending(trace, piece.cut),
                    credit_grant: rate.credit_grant,
                    once_per_product: rate.once_per_product,
                });
            }
        }
        Ok(())
    }

    /// The commitment of the minimum of `scope` for each period it is in force, prorated by its
    /// committed days, a trial's excluded, due when the period closes (R-022).
    fn commitments(&self, scope: MinimumScope, out: &mut Out) {
        let (Some(start), facts) = (self.billing_start, self.facts) else {
            return;
        };
        let start = facts.trial.map_or(start, |trial| trial.until.max(start));
        let rows: Vec<_> = facts.minimums.iter().filter(|m| m.scope == scope).collect();
        let from = rows
            .iter()
            .map(|m| m.from)
            .min()
            .map(|from| from.max(start));
        let Some(from) = from else {
            return;
        };
        let to = if rows.iter().any(|m| m.to.is_none()) {
            self.end
        } else {
            rows.iter()
                .filter_map(|m| m.to)
                .max()
                .map(|to| self.end.map_or(to, |end| end.min(to)))
        };
        if to.is_some_and(|to| to <= from) {
            return;
        }
        let lineage = scope.fee();
        for billed in self.billed((from, to), None, None) {
            let window = billed.window;
            if window.from >= window.until || self.behind_watermark(window.period) {
                continue;
            }
            let (due, cut) = self.due_at_end(&window);
            if due > self.now {
                out.wakes.push(due);
                continue;
            }
            let mut amount = 0;
            let mut trace: Vec<FactRef> = billed.plan.into_iter().collect();
            for (a, b) in self.pieces(window.from, window.until) {
                if let Some(minimum) = facts.minimum_on(scope, a) {
                    amount += Term {
                        fee: lineage.clone(),
                        full: minimum.amount,
                        period: billed.nominal,
                        from: a,
                        until: b,
                    }
                    .amount();
                    trace.push(minimum.reference());
                }
            }
            trace.sort();
            trace.dedup();
            out.charges.push(Charge {
                key: ChargeKey {
                    subscription: Some(facts.subscription),
                    fee: lineage.clone(),
                    from: window.period.0,
                    until: window.period.1,
                    kind: ChargeKind::Shortfall,
                    denomination: Denomination::Money,
                },
                window: (window.from, window.until),
                usage: None,
                quantity: None,
                amount,
                due_at: due,
                document: self.document(due, window.until == window.period.1, cut),
                trace: self.with_ending(trace, cut),
                credit_grant: None,
                once_per_product: None,
            });
        }
    }

    fn state(&self) -> DerivedState {
        let facts = self.facts;
        let today = self.today;
        let plan = facts.plan_on(today);
        let settled = facts.start.settled.is_some_and(|at| at <= self.now);
        let trial = facts
            .trial
            .filter(|trial| trial.from <= today && today < trial.until);
        let status = if let Some(ending) = self.ending.filter(|_| self.ended_by_now()) {
            match ending.kind {
                EndingKind::Cancel => Status::Cancelled,
                EndingKind::Complete => Status::Completed,
            }
        } else if self.now < midnight(facts.start.date) {
            Status::Pending
        } else if let Some(trial) = trial {
            if trial.free || settled {
                Status::Trialing
            } else {
                Status::Pending
            }
        } else if facts.free_trial().is_some_and(|trial| today >= trial.until) && !settled {
            Status::TrialExpired
        } else if self.billing_start.is_some_and(|start| today >= start)
            && (settled || facts.free_trial().is_some())
        {
            Status::Active
        } else {
            Status::Pending
        };
        let pending_checkout = !settled
            && (status == Status::TrialExpired
                || (status == Status::Pending && self.now >= midnight(facts.start.date)));
        let current_period = match status {
            Status::Cancelled | Status::Completed => self.end.map(|end| (end, None)),
            Status::Trialing if self.billing_start.is_none_or(|start| today < start) => {
                trial.map(|trial| (trial.from, Some(trial.until)))
            }
            _ => self.period_of(today).map(|(start, end)| (start, Some(end))),
        };
        DerivedState {
            status,
            current_period,
            plan_version: plan.map(|plan| plan.plan_version),
            cadence: plan.map(|plan| plan.cadence),
            pending_checkout,
            first_period: self.billing_start.and_then(|start| self.period_of(start)),
        }
    }

    /// The subscription period holding `at`, cut by the ending.
    fn period_of(&self, at: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
        let i = self
            .boundaries
            .partition_point(|b| *b <= at)
            .checked_sub(1)?;
        let start = self.boundaries[i];
        let next = self.boundaries.get(i + 1).copied();
        let end = match (next, self.end) {
            (Some(next), Some(end)) => next.min(end),
            (next, end) => next.or(end)?,
        };
        (start < end).then_some((start, end))
    }

    fn next_wake(&self, wakes: &[DateTime<Utc>]) -> Option<DateTime<Utc>> {
        let facts = self.facts;
        let boundary = self
            .boundaries
            .iter()
            .map(|b| midnight(*b))
            .find(|at| *at > self.now);
        wakes
            .iter()
            .copied()
            .chain(boundary)
            .chain(facts.trial.map(|trial| midnight(trial.until)))
            .chain(self.ending.map(|ending| ending.at))
            .chain(std::iter::once(midnight(facts.start.date)))
            .chain(self.billing_start.map(midnight))
            .chain(
                facts
                    .plan_windows
                    .iter()
                    .map(|window| midnight(window.from)),
            )
            .filter(|at| *at > self.now)
            .min()
    }
}

#[cfg(test)]
#[path = "evaluate_tests.rs"]
mod tests;
