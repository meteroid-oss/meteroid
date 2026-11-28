//! What a subscription owes at an instant, from its facts alone: every charge due by then, the
//! document it goes on, the state the facts put it in and when to look again (R-084, R-079,
//! R-016, R-011).

use std::borrow::Cow;
use std::collections::BTreeMap;

use chrono::{DateTime, Months, NaiveDate, Utc};
use common_domain::ids::{PlanVersionId, ProductId};
use rust_decimal::Decimal;

use crate::charge::{ChargeKey, ChargeKind, Denomination, Fee};
use crate::facts::{
    Billing, DatedFact, Ending, EndingKind, FactKind, FactRef, Facts, FeeFact, MinimumScope,
    SeatStep, midnight,
};
use crate::issued::IssuedLine;
use crate::pricer::{
    CreditGrant, InForce, Pricer, Rate, Segment, Shown, UsageRead, UsageReads, UsageWindow,
};
use crate::pricing::PricingError;
use crate::proration::{DayFraction, DaySpan};
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
    pub once_per_product: Option<ProductId>,
    /// How its rate was priced; a fixed fee's unit price is of a whole period.
    pub shown: Shown,
    /// The part of a whole period a prorated price bills over its window, below one.
    pub share: Option<DayFraction>,
    /// The period its price is quoted over.
    pub nominal: (NaiveDate, NaiveDate),
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
    pub plan_version: Option<PlanVersionId>,
    pub cadence: Option<u32>,
    /// The start not settled while a checkout session of it is not completed.
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
    /// The document the window's charges go on.
    pub document: DocumentSlot,
}

/// A usage window as [`evaluate`] bills it: the row pricing it, the days it bills `[from, until)`
/// (an ending's day alone when both are that day), the instants its usage is read over, the share
/// of a capacity's allowance it holds and the document its charges go on.
#[derive(Clone, Debug, PartialEq)]
pub struct BilledWindow<'a, F> {
    pub row: &'a FeeFact<F>,
    pub days: (NaiveDate, NaiveDate),
    pub read: UsageWindow,
    pub allowance: DayFraction,
    pub document: DocumentSlot,
}

/// A usage window begun by the evaluation's instant, its row by index in the facts' fees.
#[derive(Clone, Copy, Debug)]
struct Begun {
    row: usize,
    days: (NaiveDate, NaiveDate),
    read: UsageWindow,
    allowance: DayFraction,
    document: DocumentSlot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluation {
    pub charges: Vec<Charge>,
    pub state: DerivedState,
    pub next_wake: Option<DateTime<Utc>>,
    pub usage_needed: Vec<UsageNeed>,
    /// The periods of each fee the reconcile watermark settled, evaluated no more (R-094).
    pub settled: Vec<(Fee, (NaiveDate, NaiveDate))>,
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

    /// Whether `line` bills a period the watermark settled (R-094): one holds it, and no charge
    /// computed that starts on its first day bills it over a period as narrow, as a run of another
    /// cadence nested in it does.
    pub fn settles(&self, line: &IssuedLine) -> bool {
        let days = |(from, until): (NaiveDate, NaiveDate)| (until - from).num_days();
        let Some(settled) = self
            .settled
            .iter()
            .filter(|(fee, period)| line.settled_by(&[(fee.clone(), *period)]))
            .map(|(_, period)| days(*period))
            .min()
        else {
            return false;
        };
        !self.charges.iter().any(|charge| {
            line.bills(&charge.key)
                && charge.window.0 == line.from
                && days((charge.key.from, charge.key.until)) <= settled
        })
    }
}

/// The key of `keys` that `line` counts for (R-080): of those holding it, one a charge of
/// `charges` starts on the line's first day under, then one billing some of its days, then the
/// narrowest, as runs of other cadences nest in a lineage's periods.
pub fn owner<'k>(
    line: &IssuedLine,
    keys: impl IntoIterator<Item = &'k ChargeKey>,
    charges: &[Charge],
) -> Option<&'k ChargeKey> {
    let days =
        |(from, until): (NaiveDate, NaiveDate)| (from, until.max(from + chrono::Days::new(1)));
    let (from, until) = days((line.from, line.until));
    let overlaps = |key: &ChargeKey| {
        charges.iter().any(|charge| {
            let window = days(charge.window);
            charge.key == *key && window.0 < until && from < window.1
        })
    };
    let starts = |key: &ChargeKey| {
        charges
            .iter()
            .any(|charge| charge.key == *key && charge.window.0 == line.from)
    };
    keys.into_iter()
        .filter(|key| line.bills(key))
        .min_by_key(|key| {
            (
                !starts(key),
                !overlaps(key),
                (key.until - key.from).num_days(),
            )
        })
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
    run.evaluate().map(|(evaluation, _)| evaluation)
}

/// The usage windows begun by `now`, ended or still open, each with the row pricing it and the
/// document [`evaluate`] puts its charges on, read alike (R-002, R-016, R-084, R-094). Without
/// reads: what each window bills is the pricer's, on the read.
pub fn usage_windows<'a, F>(
    facts: &'a Facts<F>,
    now: DateTime<Utc>,
    pricer: &impl Pricer<F>,
) -> Result<Vec<BilledWindow<'a, F>>, PricingError> {
    let run = Run::new(facts, now, None, pricer);
    let (_, begun) = run.evaluate()?;
    Ok(begun
        .into_iter()
        .map(|window| BilledWindow {
            row: &facts.fees[window.row],
            days: window.days,
            read: window.read,
            allowance: window.allowance,
            document: window.document,
        })
        .collect())
}

/// The subscription's first renewal on or after `day` as its facts stand at `now`: the start of
/// one of its periods; None when it ends first (R-106).
pub fn renewal_on_or_after<F>(
    facts: &Facts<F>,
    day: NaiveDate,
    now: DateTime<Utc>,
) -> Option<NaiveDate> {
    let run = Run::new(facts, now, None, &Unpriced);
    run.boundaries
        .iter()
        .copied()
        .find(|boundary| *boundary >= day)
}

/// Whether a period of a fee billed every `months` months starts on `day`, on the grid of the
/// plan window in force then (R-084): a timeline change repricing it there needs no adjustment.
pub fn starts_fee_period<F>(facts: &Facts<F>, day: NaiveDate, months: u32) -> bool {
    let run = Run::new(
        facts,
        day.and_time(chrono::NaiveTime::MIN).and_utc(),
        None,
        &Unpriced,
    );
    if run.spans.is_empty() {
        return false;
    }
    run.grid(run.span_on(day), months.max(1))
        .is_some_and(|grid| grid.is_boundary(day))
}

/// Prices nothing: for the questions on the facts' periods alone.
struct Unpriced;

impl<F> Pricer<F> for Unpriced {
    fn metrics(&self, _: &F) -> Vec<common_domain::ids::BillableMetricId> {
        Vec::new()
    }

    fn rates(&self, _: &Segment<'_, F>) -> Result<Vec<Rate>, PricingError> {
        Ok(Vec::new())
    }

    fn usage(
        &self,
        _: &Segment<'_, F>,
        _: &UsageWindow,
        _: &UsageReads,
    ) -> Result<Vec<Rate>, PricingError> {
        Ok(Vec::new())
    }
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
    /// The cadence of the grid `window.period` is a period of.
    months: u32,
    /// The cadence of the grid the window itself was cut on, and its period on that grid,
    /// nested in a longer period or not.
    cadence: u32,
    own: (NaiveDate, NaiveDate),
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
    cut: bool,
    plan: Option<FactRef>,
    allowance: DayFraction,
}

struct Run<'a, F, P> {
    facts: &'a Facts<F>,
    pricer: &'a P,
    usage: Option<&'a UsageReads>,
    now: DateTime<Utc>,
    today: NaiveDate,
    settled: bool,
    billing_start: Option<NaiveDate>,
    ending: Option<Ending>,
    end: Option<NaiveDate>,
    horizon: NaiveDate,
    cuts: Vec<NaiveDate>,
    spans: Vec<PlanSpan>,
    boundaries: Vec<NaiveDate>,
    lineages: Vec<Fee>,
    /// The seat steps as billed: a removal for a period's end counted from the renewal it
    /// waits for.
    seats: Cow<'a, [SeatStep]>,
}

#[derive(Default)]
struct Out {
    charges: Vec<Charge>,
    wakes: Vec<DateTime<Utc>>,
    needed: Vec<UsageNeed>,
    settled: Vec<(Fee, (NaiveDate, NaiveDate))>,
    begun: Vec<Begun>,
}

impl<'a, F, P: Pricer<F>> Run<'a, F, P> {
    fn new(
        facts: &'a Facts<F>,
        now: DateTime<Utc>,
        usage: Option<&'a UsageReads>,
        pricer: &'a P,
    ) -> Self {
        let today = now.date_naive();
        let settled = facts.settled_by(now);
        // A cancellation before the start settled bills nothing (R-013); one still to come
        // leaves the checkout's invoice to bill.
        let cancelled_unsettled = facts.ending.is_some_and(|ending| ending.at <= now);
        // A trial awaiting a decision past its end bills nothing until one moves its end (R-167).
        let billing_start = ((settled || !cancelled_unsettled) && !facts.awaits_decision(now))
            .then(|| facts.billing_start());
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
            settled,
            billing_start,
            ending,
            end,
            horizon,
            cuts,
            spans,
            boundaries: Vec::new(),
            lineages,
            seats: Cow::Borrowed(&facts.seats),
        };
        run.boundaries = run.subscription_boundaries();
        if let Some(seats) = run.seats_at_renewal() {
            let mut cuts: Vec<NaiveDate> = facts
                .dated()
                .filter(|fact| fact.reference().kind != FactKind::Seat)
                .flat_map(|fact| fact.cuts())
                .chain(seats.iter().map(|step| step.at))
                .chain(billing_start)
                .chain(end)
                .collect();
            cuts.sort();
            cuts.dedup();
            run.cuts = cuts;
            run.seats = Cow::Owned(seats);
        }
        run
    }

    /// The seats of `lineage` on `at`: the latest step on or before it.
    fn seats_on(&self, lineage: &Fee, at: NaiveDate) -> Option<&SeatStep> {
        self.seats
            .iter()
            .filter(|step| step.lineage == *lineage && step.at <= at)
            .max_by_key(|step| (step.at, step.id))
    }

    /// The seat steps with each removal scheduled for a period's end counted from the first
    /// renewal of its fee on or after its day, its seats billed until then (R-108); None when
    /// every such removal falls on a renewal.
    fn seats_at_renewal(&self) -> Option<Vec<SeatStep>> {
        let mut lineages: Vec<&Fee> = self
            .facts
            .seats
            .iter()
            .filter(|step| step.removed_at_renewal > 0)
            .map(|step| &step.lineage)
            .collect();
        if lineages.is_empty() {
            return None;
        }
        lineages.sort();
        lineages.dedup();
        let mut moved = false;
        let mut seats: Vec<SeatStep> = self
            .facts
            .seats
            .iter()
            .filter(|step| !lineages.contains(&&step.lineage))
            .cloned()
            .collect();
        for lineage in lineages {
            let runs = self.renewals(lineage);
            let mut raw: Vec<&SeatStep> = self
                .facts
                .seats
                .iter()
                .filter(|step| step.lineage == *lineage)
                .collect();
            raw.sort_by_key(|step| (step.at, step.id));
            // Each removal with the renewal it waits for: the first of the run billing its day, else
            // that run's end. Outside any run, the count is the next step's to set.
            let waits: Vec<(&SeatStep, NaiveDate)> = raw
                .iter()
                .filter(|step| step.removed_at_renewal > 0)
                .map(|step| {
                    let renewal = runs
                        .iter()
                        .find(|(from, until, _)| *from <= step.at && step.at < *until)
                        .map_or(step.at, |(_, until, renewals)| {
                            renewals
                                .iter()
                                .copied()
                                .find(|day| *day >= step.at)
                                .unwrap_or(*until)
                        });
                    moved |= renewal != step.at;
                    (*step, renewal)
                })
                .collect();
            let mut days: Vec<(NaiveDate, &SeatStep)> = raw
                .iter()
                .map(|step| (step.at, *step))
                .chain(waits.iter().map(|(step, renewal)| (*renewal, *step)))
                .collect();
            days.sort_by_key(|(day, step)| (*day, step.at != *day, step.id));
            days.dedup_by_key(|(day, _)| *day);
            let mut last: Option<u64> = None;
            for (day, step) in days {
                let Some(latest) = raw.iter().rev().find(|step| step.at <= day) else {
                    continue;
                };
                let waiting: u64 = waits
                    .iter()
                    .filter(|(step, renewal)| step.at <= day && day < *renewal)
                    .map(|(step, _)| step.removed_at_renewal)
                    .sum();
                let quantity = latest.quantity.saturating_add(waiting);
                if last == Some(quantity) {
                    continue;
                }
                last = Some(quantity);
                seats.push(SeatStep {
                    at: day,
                    quantity,
                    removed_at_renewal: 0,
                    ..step.clone()
                });
            }
        }
        moved.then_some(seats)
    }

    /// Each run of `lineage` billing by period, `[from, until)`, with the days it renews on: each
    /// window of it opening its period.
    fn renewals(&self, lineage: &Fee) -> Vec<(NaiveDate, NaiveDate, Vec<NaiveDate>)> {
        let intervals = self.intervals(lineage);
        self.runs(&intervals)
            .into_iter()
            .filter(|(rows, _)| rows[0].row.billing != Billing::OneTime)
            .filter_map(|(_, billed)| {
                let (first, last) = (billed.first()?, billed.last()?);
                let renewals = billed
                    .iter()
                    .map(|billed| billed.window)
                    .filter(|window| window.from == window.period.0)
                    .map(|window| window.from)
                    .collect();
                Some((first.window.from, last.window.until, renewals))
            })
            .collect()
    }

    /// `intervals` in runs of rows billing alike, each with the windows it bills (R-084).
    fn runs<'b>(
        &self,
        intervals: &'b [Interval<'a, F>],
    ) -> Vec<(&'b [Interval<'a, F>], Vec<Billed>)> {
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
        let mut previous: Option<Billed> = None;
        runs.into_iter()
            .map(|(first, last)| {
                let rows = &intervals[first..=last];
                let billing = rows[0].row.billing;
                let life = (rows[0].from, rows[rows.len() - 1].to);
                let months = (billing != Billing::OneTime).then_some(rows[0].row.cadence.max(1));
                // A run of another cadence on the lineage bills on its own grid, keyed on its
                // own periods (R-084).
                let before =
                    previous.filter(|last| months.is_none_or(|months| last.cadence == months));
                let billed = self.billed(life, months, before);
                previous = billed.last().copied().or(previous);
                (rows, billed)
            })
            .collect()
    }

    /// The evaluation, with the usage windows begun by its instant.
    fn evaluate(&self) -> Result<(Evaluation, Vec<Begun>), PricingError> {
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
            mut settled,
            mut begun,
            ..
        } = out;
        if let Some(start) = self.billing_start.filter(|start| self.end == Some(*start)) {
            // Ended the day it started billing: with no fee billing in advance, no invoice opened
            // that day, so the ending's charges open it (R-016).
            let opening = DocumentSlot::Recurring {
                period_start: start,
            };
            let opened = self.lineages.iter().any(|lineage| {
                self.facts
                    .fee_on(lineage, start)
                    .is_some_and(|row| row.billing != Billing::Arrears)
            });
            if !opened {
                for charge in &mut charges {
                    if charge.document == DocumentSlot::Final {
                        charge.document = opening;
                    }
                }
                for need in &mut needed {
                    if need.document == DocumentSlot::Final {
                        need.document = opening;
                    }
                }
                for window in &mut begun {
                    if window.document == DocumentSlot::Final {
                        window.document = opening;
                    }
                }
            }
        }
        if let (false, Some(start)) = (self.settled, self.billing_start) {
            // Until its start settles, a subscription bills its checkout's invoice alone, which
            // bills no arrears (R-011).
            let opening = DocumentSlot::Recurring {
                period_start: start,
            };
            charges.retain(|charge| charge.document == opening);
            needed.clear();
            begun.clear();
        }
        charges.sort_by(|a, b| {
            (a.due_at, &a.key, a.window, a.usage, a.amount)
                .cmp(&(b.due_at, &b.key, b.window, b.usage, b.amount))
        });
        needed.sort();
        needed.dedup();
        settled.sort();
        settled.dedup();
        begun.sort_by_key(|window| (window.read, window.days, window.row));
        Ok((
            Evaluation {
                charges,
                state,
                next_wake,
                usage_needed: needed,
                settled,
            },
            begun,
        ))
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

    /// A cancellation requested after the day it ends: no document is dated before it was requested.
    fn backdated(&self) -> bool {
        self.ending
            .is_some_and(|ending| ending.kind == EndingKind::Cancel)
            && self
                .end
                .zip(self.facts.ending_recorded)
                .is_some_and(|(end, recorded)| recorded > end)
    }

    /// The document of the last charges of an ending: a final invoice of its own when the period
    /// opened that day was invoiced before it (an immediate cancellation, or one decided after
    /// that invoice was written) or the cancellation is backdated, else the recurring invoice
    /// opened on its date (R-016, R-102).
    fn ending_slot(&self) -> DocumentSlot {
        let (Some(ending), Some(end)) = (self.ending, self.end) else {
            return DocumentSlot::Adjustment { on: self.today };
        };
        let after_its_invoice = ending.at > midnight(end)
            || (ending.kind == EndingKind::Cancel && self.facts.ending_day_invoiced);
        if self.backdated() || (self.is_boundary(end) && after_its_invoice) {
            DocumentSlot::Final
        } else {
            DocumentSlot::Recurring { period_start: end }
        }
    }

    /// A charge due on a boundary goes on that period's recurring invoice; one `riding` between
    /// two boundaries (its fee's own grid, arrears ended with their fee) the next renewal's, the
    /// first past the watermark when that one was issued before it (R-094), as the store bills
    /// them; an advance due at a change mid-period on an adjustment invoice. A backdated
    /// cancellation closes the recurring drafts from its ending: what they would bill, the last
    /// period's arrears included, goes on its final invoice; one it keeps keeps its lines (R-016).
    fn document(&self, due: DateTime<Utc>, riding: bool, cut: bool) -> DocumentSlot {
        match self.slot(due, riding, cut) {
            DocumentSlot::Recurring { period_start }
                if self.backdated()
                    && self.end.is_some_and(|end| {
                        period_start > end
                            || (period_start == end && !self.facts.ending_day_invoiced)
                    }) =>
            {
                DocumentSlot::Final
            }
            slot => slot,
        }
    }

    fn slot(&self, due: DateTime<Utc>, riding: bool, cut: bool) -> DocumentSlot {
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
        if riding {
            let next = |after: NaiveDate| {
                let i = self.boundaries.partition_point(|b| *b <= after);
                self.boundaries.get(i).copied()
            };
            let past_watermark = self
                .arrears_watermark()
                .filter(|watermark| *watermark > on)
                .and_then(next);
            if let Some(next) = past_watermark.or_else(|| next(on)) {
                return DocumentSlot::Recurring { period_start: next };
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
    /// What a new grid puts inside the period the run (or one of its cadence before it) is billing
    /// stays keyed on that period; a window of another cadence is priced over its own period.
    fn billed(
        &self,
        (from, to): (NaiveDate, Option<NaiveDate>),
        months: Option<u32>,
        before: Option<Billed>,
    ) -> Vec<Billed> {
        let mut billed: Vec<Billed> = Vec::new();
        // A run keeps the grid of the plan window it started in: a change of cadence moves only
        // the rows it writes onto its grid (R-084).
        let started_in = *self.span_on(from);
        let cadence = months.unwrap_or(started_in.cadence);
        let Some(grid) = self.grid(&started_in, cadence) else {
            return billed;
        };
        for span in &self.spans {
            let a = span.from.map_or(from, |s| s.max(from));
            let b = match (span.to, to) {
                (Some(x), Some(y)) => Some(x.min(y)),
                (x, y) => x.or(y),
            };
            if b.is_some_and(|b| b <= a) {
                continue;
            }
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
                let alike = inside.is_some_and(|last| {
                    months.is_none() || (last.cadence == cadence && last.own == window.period)
                });
                match (inside, billed.last_mut()) {
                    (Some(_), Some(last)) if alike && last.window.until == window.from => {
                        last.window.until = window.until;
                    }
                    (Some(last), _) => billed.push(Billed {
                        window: Window {
                            period: last.window.period,
                            ..window
                        },
                        nominal: if alike {
                            last.nominal
                        } else {
                            grid.nominal(window.period)
                        },
                        plan: span.plan,
                        months: last.months,
                        cadence,
                        own: window.period,
                    }),
                    (None, _) => billed.push(Billed {
                        window,
                        nominal: grid.nominal(window.period),
                        plan: span.plan,
                        months: cadence,
                        cadence,
                        own: window.period,
                    }),
                }
            }
        }
        billed
    }

    /// Whether a plan window starting on `on` moves the subscription onto another plan version.
    fn plan_version_changes_on(&self, on: NaiveDate) -> bool {
        let windows = &self.facts.plan_windows;
        windows.iter().filter(|w| w.from == on).any(|window| {
            windows
                .iter()
                .filter(|w| w.from < on)
                .max_by_key(|w| w.from)
                .is_none_or(|before| before.plan_version != window.plan_version)
        })
    }

    /// Whether `period` ends by the reconcile watermark: its charges are settled for good.
    fn behind_watermark(&self, period: (NaiveDate, NaiveDate)) -> bool {
        self.facts
            .reconcile_from
            .is_some_and(|watermark| period.1 <= watermark)
    }

    /// The watermark of what a period bills at its end: never later than the watermark.
    fn arrears_watermark(&self) -> Option<NaiveDate> {
        let watermark = self.facts.reconcile_from?;
        Some(
            self.facts
                .arrears_from
                .map_or(watermark, |from| from.min(watermark)),
        )
    }

    /// As [`Self::behind_watermark`], for what `period` bills at its end.
    fn arrears_behind_watermark(&self, period: (NaiveDate, NaiveDate)) -> bool {
        self.arrears_watermark()
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
        if intervals.is_empty() {
            self.one_time_in_trial(lineage, out)?;
        }
        let mut one_time_billed = false;
        let mut last_run: Option<(NaiveDate, &FeeFact<F>)> = None;
        for (rows, billed) in self.runs(&intervals) {
            last_run = Some((rows[0].from, rows[rows.len() - 1].row));
            let billing = rows[0].row.billing;
            let life = (rows[0].from, rows[rows.len() - 1].to);
            if billing == Billing::OneTime {
                // A plan's one-time fee bills with the start, never with a later plan version.
                let with_plan = Some(life.0) != self.billing_start
                    && rows[0].row.plan_component
                    && self.plan_version_changes_on(life.0);
                if !one_time_billed
                    && !with_plan
                    && let Some(first) = billed.first()
                {
                    let rates = || self.rates_on(lineage, first.window.from, prices);
                    self.one_time(lineage, first, rates, out)?;
                }
                one_time_billed = true;
                continue;
            }
            for window in &billed {
                self.fixed(lineage, billing, window, prices, out)?;
                self.usage_window(rows, window, usage_pieces, out);
            }
        }
        self.final_usage(lineage, last_run, usage_pieces, out);
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
            seats: self.seats_on(&row.lineage, at).map(|step| step.quantity),
            fee: &row.fee,
            allowance: DayFraction::WHOLE,
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
            .chain(self.seats_on(lineage, at).map(|step| step.reference()))
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

    /// Whether `billed` opens the billing in a period it holds only the end of (R-002).
    fn first_partial(&self, billed: &Billed) -> bool {
        Some(billed.window.from) == self.billing_start && billed.window.from > billed.nominal.0
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
        let behind = match billing {
            Billing::Arrears => self.arrears_behind_watermark(window.period),
            _ => self.behind_watermark(window.period),
        };
        if behind {
            out.settled.push((lineage.clone(), window.period));
            return Ok(());
        }
        if window.from >= window.until {
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
                    // Cut short by its rows, a cancellation or a fixed term's end, or a first partial
                    // period, it owes what it used, a fixed amount too (R-002).
                    let cut_short = window.until < window.period.1 || self.first_partial(billed);
                    let used: Vec<(NaiveDate, NaiveDate, Option<Rate>)> = terms
                        .iter()
                        .map(|(a, b, rate)| {
                            let rate = rate.map(|rate| Rate {
                                prorated: rate.prorated
                                    || (cut_short && rate.kind != ChargeKind::Credit),
                                ..rate.clone()
                            });
                            (*a, *b, rate)
                        })
                        .collect();
                    let prorated = used.iter().all(|t| t.2.as_ref().is_none_or(|r| r.prorated));
                    let amount = if prorated {
                        used.iter()
                            .map(|(a, b, rate)| part(rate.as_ref(), *a, *b))
                            .sum()
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
                    let single = terms.len() == 1;
                    out.charges.push(Charge {
                        key: key(charge),
                        window: (window.from, window.until),
                        usage: None,
                        quantity: last.and_then(|r| r.quantity),
                        amount,
                        due_at: due,
                        document: self.document(due, true, cut),
                        trace: self.with_ending(trace, cut),
                        credit_grant: grant(last, grant_amount),
                        once_per_product: last.and_then(|r| r.once_per_product),
                        shown: last.map(|r| r.shown.clone()).unwrap_or_default(),
                        share: last
                            .filter(|_| single && prorated)
                            .and_then(|_| share_of((window.from, window.until), billed.nominal)),
                        nominal: billed.nominal,
                    });
                }
                _ => {
                    // Ended inside its period by its rows, a cancellation or a fixed term, or a first
                    // partial period: what it used is prorated, a fixed amount too (R-002, R-077).
                    let cut_short = window.until < window.period.1 || self.first_partial(billed);
                    for (i, (from, _, rate)) in terms.iter().enumerate() {
                        let previous = i.checked_sub(1).and_then(|p| terms[p].2);
                        // A change mid-period prorates a fixed amount too (R15); a pack is bought
                        // whole, keyed on its day as its line is (R-075).
                        let changed = i > 0
                            || (*from != window.period.0 && Some(*from) != self.billing_start);
                        let changing = |rate: Option<&Rate>| {
                            rate.map(|rate| Rate {
                                prorated: rate.prorated
                                    || ((changed || cut_short) && rate.kind != ChargeKind::Credit),
                                ..rate.clone()
                            })
                        };
                        // A pack is never netted: it bills its own price, the one it replaces
                        // or that ends given back by its credit's settlement (OQ1).
                        let pack = charge.0 == ChargeKind::Credit;
                        let (now_in, before) = (changing(*rate), changing(previous));
                        let before = before.filter(|_| !pack);
                        let amount = part(now_in.as_ref(), *from, window.until)
                            - part(before.as_ref(), *from, window.until);
                        let grant_amount = granted(*rate, *from, window.until)
                            - if pack {
                                0
                            } else {
                                granted(previous, *from, window.until)
                            };
                        if ((i == 0 || pack) && rate.is_none())
                            || (i > 0 && amount == 0 && grant_amount == 0)
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
                        let mut key = key(charge.clone());
                        let mut days = (*from, window.until);
                        if changed
                            && priced.is_some_and(|r| r.kind == ChargeKind::Credit && !r.prorated)
                        {
                            (key.from, key.until) = (*from, *from);
                            days = (*from, *from);
                        }
                        let prorated = i == 0 && now_in.as_ref().is_some_and(|r| r.prorated);
                        out.charges.push(Charge {
                            key,
                            window: days,
                            usage: None,
                            quantity: priced.and_then(|r| r.quantity),
                            amount,
                            due_at: due,
                            document: self.document(due, on_grid, false),
                            trace: self.trace(lineage, *from, billed.plan),
                            credit_grant: grant(priced, grant_amount),
                            once_per_product: priced.and_then(|r| r.once_per_product),
                            shown: priced.map(|r| r.shown.clone()).unwrap_or_default(),
                            share: prorated
                                .then(|| share_of((*from, window.until), billed.nominal))
                                .flatten(),
                            nominal: billed.nominal,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// A one-time fee added in a free trial and living to its end bills then, though an ending or
    /// a change closing the trial leaves its row no day to bill: it lived before them (R-093,
    /// R-016). The day is the ending's when it closes the trial. A plan's setup is left out.
    fn one_time_in_trial(&self, lineage: &Fee, out: &mut Out) -> Result<(), PricingError> {
        let Some(start) = self.billing_start else {
            return Ok(());
        };
        let day = self.end.map_or(start, |end| end.min(start));
        let row = self
            .facts
            .fees
            .iter()
            .filter(|row| {
                row.lineage == *lineage
                    && row.billing == Billing::OneTime
                    && !row.plan_component
                    && row.from < day
                    && row.to.is_none_or(|to| to >= day)
            })
            .min_by_key(|row| row.from);
        let Some(row) = row else {
            return Ok(());
        };
        let period = (day, day.succ_opt().unwrap_or(day));
        let billed = Billed {
            window: Window {
                period,
                from: day,
                until: day,
            },
            nominal: period,
            plan: self.span_on(day).plan,
            months: 1,
            cadence: 1,
            own: period,
        };
        let rates = || {
            let segment = Segment {
                from: day,
                until: Some(day),
                fees: vec![self.in_force(row, day)],
            };
            self.pricer.rates(&segment)
        };
        let before = out.charges.len();
        self.one_time(lineage, &billed, rates, out)?;
        // A row a change ended that day is no longer in force on it: its line names it.
        for charge in &mut out.charges[before..] {
            charge.trace.push(row.reference());
            charge.trace.sort();
            charge.trace.dedup();
        }
        Ok(())
    }

    /// A one-time fee bills once, at the start of its life.
    fn one_time(
        &self,
        lineage: &Fee,
        billed: &Billed,
        rates: impl FnOnce() -> Result<Vec<Rate>, PricingError>,
        out: &mut Out,
    ) -> Result<(), PricingError> {
        let window = billed.window;
        if self.behind_watermark(window.period) {
            out.settled.push((lineage.clone(), window.period));
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
        for rate in rates()?.into_iter().filter(|rate| rate.fee == *lineage) {
            // A one-time price bills on its day, as its line is keyed (R-075), a pack bought whole
            // after the start too; one tied to its period grants for its first window.
            let on_its_day = rate.kind != ChargeKind::Credit
                || (!rate.prorated && Some(window.from) != self.billing_start);
            let (key, days) = if on_its_day {
                ((window.from, window.from), (window.from, window.from))
            } else {
                (window.period, (window.from, window.until))
            };
            out.charges.push(Charge {
                key: ChargeKey {
                    subscription: Some(self.facts.subscription),
                    fee: lineage.clone(),
                    from: key.0,
                    until: key.1,
                    kind: rate.kind,
                    denomination: rate.denomination,
                },
                window: days,
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
                share: rate
                    .prorated
                    .then(|| share_of(days, billed.nominal))
                    .flatten(),
                shown: rate.shown,
                nominal: billed.nominal,
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
        let document = self.document(piece.read.until, true, piece.cut);
        if piece.read.from <= self.now
            && let Some(row) = self
                .facts
                .fees
                .iter()
                .position(|row| std::ptr::eq(row, piece.row))
        {
            out.begun.push(Begun {
                row,
                days: (piece.from, piece.until),
                read: piece.read,
                allowance: piece.allowance,
                document,
            });
        }
        let need = |read: UsageRead| UsageNeed {
            fee: piece.row.lineage.clone(),
            read,
            document,
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
        if self.arrears_behind_watermark(window.period) {
            if let Some(interval) = rows.first() {
                out.settled
                    .push((interval.row.lineage.clone(), window.period));
            }
            return;
        }
        // Cut inside its own grid's period, whatever period it is keyed on: ending on its own
        // boundary, the ending's day is the final usage's (R-016).
        let cut = self.end == Some(window.until) && window.until < billed.nominal.1;
        let mut pieces = Vec::new();
        for interval in rows {
            let from = interval.from.max(window.from);
            let until = interval.to.map_or(window.until, |to| to.min(window.until));
            if from >= until {
                continue;
            }
            // Only the row still in force on the ending's day reads its morning; another row
            // taking over that day leaves it to the final usage.
            let in_force_then = self
                .end
                .and_then(|end| self.facts.fee_on(&interval.row.lineage, end))
                .is_some_and(|row| std::ptr::eq(row, interval.row));
            let to_instant = cut && until == window.until && in_force_then;
            let read_until = match (to_instant, self.ending) {
                (true, Some(ending)) => ending.at,
                _ => midnight(until),
            };
            let read_from = interval
                .row
                .usage_from
                .map_or(midnight(from), |floor| floor.max(midnight(from)));
            pieces.push(UsagePiece {
                row: interval.row,
                from,
                until,
                read: UsageWindow {
                    from: read_from.min(read_until),
                    until: read_until,
                },
                cut: cut && until == window.until,
                plan: billed.plan,
                allowance: DayFraction::WHOLE,
            });
        }
        let Some(last) = pieces.last() else {
            return;
        };
        if !self.pricer.per_part(&last.row.fee) {
            // Rows sharing one allowance price the period once; a row changing it (a capacity's
            // level) starts a part of its own, each part's allowance its share of the days (R-135).
            let runs: Vec<&[UsagePiece<'a, F>]> = pieces
                .chunk_by(|a, b| self.pricer.shares_allowance(&a.row.fee, &b.row.fee))
                .collect();
            let several = runs.len() > 1;
            // Its allowance follows its fixed amount: prorated when the window is cut short, is a
            // first partial period, or starts after its period does in advance (R-002).
            let starts_late =
                window.from > window.period.0 && Some(window.from) != self.billing_start;
            let prorated = several
                || window.until < window.period.1
                || self.first_partial(billed)
                || (last.row.billing == Billing::Advance && starts_late);
            let merged: Vec<UsagePiece<'a, F>> = runs
                .into_iter()
                .map(|run| {
                    let (first, last) = (&run[0], &run[run.len() - 1]);
                    UsagePiece {
                        row: last.row,
                        from: first.from,
                        until: last.until,
                        read: UsageWindow {
                            from: first.read.from,
                            until: last.read.until,
                        },
                        cut: last.cut,
                        plan: last.plan,
                        allowance: if prorated {
                            DaySpan {
                                period: billed.nominal,
                                from: first.from,
                                until: last.until,
                            }
                            .fraction()
                        } else {
                            DayFraction::WHOLE
                        },
                    }
                })
                .collect();
            pieces = merged;
        } else if pieces.len() > 1 {
            // A row priced once per period replaced by one priced part by part is a part of its
            // own: its allowance is its share of the days (R-135).
            for piece in pieces.iter_mut() {
                if !self.pricer.per_part(&piece.row.fee) {
                    piece.allowance = DaySpan {
                        period: billed.nominal,
                        from: piece.from,
                        until: piece.until,
                    }
                    .fraction();
                }
            }
        }
        for piece in pieces {
            self.queue(piece, usage_pieces, out);
        }
    }

    /// What `lineage` consumed on the ending's day up to its instant, priced by the row in force
    /// that day, when no window of a period cut by the ending reads it: the ending falls on a
    /// boundary of that row's grid (its run's, `last_run`), or another row billed the day before.
    fn final_usage(
        &self,
        lineage: &Fee,
        last_run: Option<(NaiveDate, &FeeFact<F>)>,
        usage_pieces: &mut Vec<UsagePiece<'a, F>>,
        out: &mut Out,
    ) {
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
        let continues = last_run.and_then(|(from, last)| std::ptr::eq(last, row).then_some(from));
        let Some(grid) = self.grid(self.span_on(continues.unwrap_or(end)), row.cadence.max(1))
        else {
            return;
        };
        if grid.period_containing(end).is_none_or(|period| {
            (period.0 != end && continues.is_some()) || self.arrears_behind_watermark(period)
        }) {
            return;
        }
        self.queue(
            UsagePiece {
                row,
                from: end,
                until: end,
                read: UsageWindow {
                    from: row
                        .usage_from
                        .map_or(midnight(end), |floor| floor.max(midnight(end)))
                        .min(ending.at),
                    until: ending.at,
                },
                cut: true,
                plan: span.plan,
                // The ending's day is no day of its fee's: a capacity's allowance holds none of it.
                allowance: DayFraction::NONE,
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
                    .map(|piece| InForce {
                        allowance: piece.allowance,
                        ..self.in_force(piece.row, piece.from)
                    })
                    .collect(),
            };
            for rate in self.pricer.usage(&segment, &window, reads)? {
                // Credits are drawn, never given back: usage netting negative in a credit unit
                // bills nothing.
                if rate.amount < 0 && matches!(rate.denomination, Denomination::CreditUnit(_)) {
                    continue;
                }
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
                    document: self.document(window.until, true, piece.cut),
                    trace: self.with_ending(trace, piece.cut),
                    credit_grant: rate.credit_grant,
                    once_per_product: rate.once_per_product,
                    shown: rate.shown,
                    share: None,
                    nominal: (piece.from, piece.until),
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
                shown: Shown::default(),
                share: None,
                nominal: billed.nominal,
            });
        }
    }

    fn state(&self) -> DerivedState {
        let facts = self.facts;
        let today = self.today;
        let plan = facts.plan_on(today);
        let settled = self.settled;
        let trial = facts.trial.filter(|trial| trial.holds_on(today));
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
        let pending_checkout = !settled && facts.start.checkout;
        let current_period = match status {
            Status::Cancelled | Status::Completed => self.end.map(|end| (end, None)),
            // Started and still to settle, the first period is open-ended, as stored until then.
            Status::Pending if !settled && self.now >= midnight(facts.start.date) => {
                self.billing_start.map(|start| (start, None))
            }
            Status::Trialing if self.billing_start.is_none_or(|start| today < start) => {
                trial.map(|trial| (trial.from, (today < trial.until).then_some(trial.until)))
            }
            _ => self
                .current_of(today)
                .map(|(start, end)| (start, Some(self.uncancelled_end(today, end)))),
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

    /// The period the subscription is in at `at`: after a change of cadence inside a period, the
    /// part of it the new grid's period holding `at` covers, as the change realigns it (R-084).
    fn current_of(&self, at: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
        let (start, end) = self.period_of(at)?;
        let span = self.span_on(at);
        if span.from.is_none_or(|from| from <= start) {
            return Some((start, end));
        }
        let (from, until) = self.grid(span, span.cadence)?.period_containing(at)?;
        Some((from.max(start), until.min(end)))
    }

    /// A cancellation still to come ends the subscription on its date, not the period holding
    /// `at`: its end is the grid's, as stored until the cancellation applies.
    fn uncancelled_end(&self, at: NaiveDate, end: NaiveDate) -> NaiveDate {
        match self.ending {
            Some(ending) if ending.kind == EndingKind::Cancel && self.end == Some(end) => {
                let span = self.span_on(at);
                self.grid(span, span.cadence)
                    .and_then(|grid| grid.period_containing(at))
                    .map_or(end, |(_, until)| until.max(end))
            }
            _ => end,
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

/// The days of `window` over those of `nominal`, when fewer (R-002).
fn share_of(
    window: (NaiveDate, NaiveDate),
    nominal: (NaiveDate, NaiveDate),
) -> Option<DayFraction> {
    let days = (window.1 - window.0).num_days();
    let whole = (nominal.1 - nominal.0).num_days();
    (whole > 0 && days < whole).then(|| DayFraction::new(days, whole))
}

#[cfg(test)]
#[path = "evaluate_tests.rs"]
mod tests;
