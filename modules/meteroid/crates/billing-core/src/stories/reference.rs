//! What a story's fees owe over its life, in plain day arithmetic (R-079, R-001, R-002): each
//! part of a period at a price bills what is left at its start less at its end, rounded once.
//! Only the calendar (R-084's grids) comes from `timeline`; no amount comes from the engine.

use std::collections::BTreeMap;

use chrono::{Days, NaiveDate};

use super::events::Events;
use super::story::{Built, Kind, Price, grid_for, midnight};
use crate::charge::Fee;
use crate::facts::{Billing, Facts, FeeFact};
use crate::timeline::{Grid, windows};

/// `num / den` rounded half away from zero, in plain integers (R-001).
fn round(num: i128, den: i128) -> i64 {
    let (q, r) = (num / den, num % den);
    let q = if 2 * r.abs() >= den {
        q + num.signum()
    } else {
        q
    };
    i64::try_from(q).expect("an amount")
}

/// What is left at `at` of `price` for the period `nominal`, rounded once.
fn left(price: i64, nominal: (NaiveDate, NaiveDate), at: NaiveDate) -> i64 {
    let whole = (nominal.1 - nominal.0).num_days();
    if whole <= 0 {
        return 0;
    }
    let days = (nominal.1 - at.clamp(nominal.0, nominal.1)).num_days();
    round(i128::from(price) * i128::from(days), i128::from(whole))
}

type Period = (NaiveDate, NaiveDate);

/// A part of a period a fee bills: the charge's period, the period its price is of, its days.
#[derive(Clone, Copy, Debug)]
struct Win {
    key: Period,
    nominal: Period,
    months: u32,
    /// The cadence of the grid the part was cut on, and its period there.
    cadence: u32,
    own: Period,
    from: NaiveDate,
    until: NaiveDate,
}

/// Consecutive days a lineage bills alike: one billing, one cadence.
#[derive(Clone, Copy, Debug)]
struct Run {
    from: NaiveDate,
    until: NaiveDate,
    billing: Billing,
    cadence: u32,
}

struct Span {
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    origin: NaiveDate,
    cadence: u32,
}

pub struct Reference<'a> {
    facts: &'a Facts<Price>,
    start: NaiveDate,
    end: NaiveDate,
    spans: Vec<Span>,
}

impl<'a> Reference<'a> {
    /// The facts of an ended story: everything they bill falls before their ending.
    pub fn new(facts: &'a Facts<Price>) -> Self {
        let end = facts.effective_ending().expect("an ended story").on();
        Self::until(facts, end)
    }

    /// The facts as known at an instant, read up to their ending, else up to `end`.
    fn until(facts: &'a Facts<Price>, end: NaiveDate) -> Self {
        let end = facts.effective_ending().map_or(end, |ending| ending.on());
        let mut plans: Vec<_> = facts.plan_windows.iter().collect();
        plans.sort_by_key(|window| (window.from, window.plan_version));
        let spans = plans
            .iter()
            .enumerate()
            .map(|(i, window)| Span {
                from: (i > 0).then_some(window.from),
                to: plans.get(i + 1).map(|next| next.from),
                origin: window.cadence_origin,
                cadence: window.cadence.max(1),
            })
            .collect();
        Reference {
            facts,
            start: facts.billing_start(),
            end,
            spans,
        }
    }

    /// Each lineage's total over the subscription's life.
    pub fn totals(&self, events: &Events) -> BTreeMap<Fee, i64> {
        let mut lineages: Vec<Fee> = self
            .facts
            .fees
            .iter()
            .map(|row| row.lineage.clone())
            .collect();
        lineages.sort();
        lineages.dedup();
        lineages
            .into_iter()
            .map(|lineage| {
                let total = self.fixed(&lineage) + self.usage(&lineage, events);
                (lineage, total)
            })
            .collect()
    }

    fn row(&self, lineage: &Fee, day: NaiveDate) -> Option<&'a FeeFact<Price>> {
        self.facts.fee_on(lineage, day)
    }

    /// The row's price for `day`: per seat for seats, whatever the seat count before the first
    /// step is (one).
    fn price(&self, row: &FeeFact<Price>, day: NaiveDate) -> i64 {
        let seats = if row.fee.kind == Kind::Seats {
            self.facts
                .seats_on(&row.lineage, day)
                .map_or(1, |step| step.quantity)
        } else {
            1
        };
        row.fee.amount * i64::try_from(seats).expect("seats")
    }

    fn days(&self) -> impl Iterator<Item = NaiveDate> + '_ {
        let mut day = self.start;
        std::iter::from_fn(move || {
            (day < self.end).then(|| {
                let today = day;
                day = day + Days::new(1);
                today
            })
        })
    }

    fn runs(&self, lineage: &Fee) -> Vec<Run> {
        let mut runs: Vec<Run> = Vec::new();
        for day in self.days() {
            let Some(row) = self.row(lineage, day) else {
                continue;
            };
            let cadence = row.cadence.max(1);
            match runs.last_mut() {
                Some(run)
                    if run.until == day && run.billing == row.billing && run.cadence == cadence =>
                {
                    run.until = day + Days::new(1);
                }
                _ => runs.push(Run {
                    from: day,
                    until: day + Days::new(1),
                    billing: row.billing,
                    cadence,
                }),
            }
        }
        runs
    }

    fn grid(&self, span: &Span, months: u32) -> Option<Grid> {
        grid_for(self.facts, span.origin, months)
    }

    /// The parts of periods `run` bills, on the grid of the plan window it started in: a change of
    /// cadence moves only the rows it writes (R-084). A part inside the period the lineage is
    /// billing stays that period's charge, unless a run of another cadence bills it (R-084).
    fn windows(&self, run: &Run, current: &mut Option<Win>) -> Vec<Win> {
        let mut out: Vec<Win> = Vec::new();
        if current.is_some_and(|win| win.cadence != run.cadence) {
            *current = None;
        }
        let started_in = self
            .spans
            .iter()
            .rev()
            .find(|span| span.from.is_none_or(|from| from <= run.from))
            .unwrap_or(&self.spans[0]);
        let Some(grid) = self.grid(started_in, run.cadence) else {
            return out;
        };
        for span in &self.spans {
            let a = span.from.map_or(run.from, |from| from.max(run.from));
            let b = span.to.map_or(run.until, |to| to.min(run.until));
            if b <= a {
                continue;
            }
            for window in windows(&grid, (a, Some(b)), b) {
                let before = out.last().copied().or(*current);
                let inside =
                    before.filter(|win| win.key.0 <= window.from && window.until <= win.key.1);
                // Alike: a part of the same period of the same grid, priced as it is.
                let alike = inside
                    .is_some_and(|win| win.cadence == run.cadence && win.own == window.period);
                match (inside, out.last_mut()) {
                    (Some(_), Some(last)) if alike && last.until == window.from => {
                        last.until = window.until;
                    }
                    (Some(win), _) => out.push(Win {
                        key: win.key,
                        nominal: if alike {
                            win.nominal
                        } else {
                            grid.nominal(window.period)
                        },
                        months: win.months,
                        cadence: run.cadence,
                        own: window.period,
                        from: window.from,
                        until: window.until,
                    }),
                    (None, _) => out.push(Win {
                        key: window.period,
                        nominal: grid.nominal(window.period),
                        months: run.cadence,
                        cadence: run.cadence,
                        own: window.period,
                        from: window.from,
                        until: window.until,
                    }),
                }
            }
        }
        if let Some(last) = out.last() {
            *current = Some(*last);
        }
        out
    }

    /// `[from, until)` cut where its price changes, with each part's price.
    fn terms(&self, lineage: &Fee, win: &Win) -> Vec<(NaiveDate, i64)> {
        let mut terms: Vec<(NaiveDate, i64)> = Vec::new();
        let mut day = win.from;
        while day < win.until {
            if let Some(row) = self.row(lineage, day) {
                let price = self.price(row, day);
                if terms.last().is_none_or(|(_, last)| *last != price) {
                    terms.push((day, price));
                }
            }
            day = day + Days::new(1);
        }
        terms
    }

    /// What a window bills of `kind`.
    fn window_amount(&self, lineage: &Fee, kind: Kind, win: &Win) -> i64 {
        let terms = self.terms(lineage, win);
        let Some(&(first_day, first_price)) = terms.first() else {
            return 0;
        };
        let w = win.until;
        let next = |i: usize| terms.get(i + 1).map_or(w, |(day, _)| *day);
        let part = |price: i64, from: NaiveDate, until: NaiveDate| {
            left(price, win.nominal, from) - left(price, win.nominal, until)
        };
        match kind {
            // R-079: each price for its days, the parts tiling the window.
            Kind::Fixed | Kind::Seats => terms
                .iter()
                .enumerate()
                .map(|(i, (day, price))| part(*price, *day, next(i)))
                .sum(),
            // R-002: whole at the period's price, unless a change starts it inside its period,
            // the billing starts in it, or its rows, a cancellation or a fixed term end it there.
            Kind::Capacity => {
                let cut_short =
                    w < win.key.1 || (win.from == self.start && win.from > win.nominal.0);
                let changed = first_day != win.key.0 && first_day != self.start;
                let mut total = if changed || cut_short {
                    part(first_price, first_day, w)
                } else {
                    first_price
                };
                for pair in terms.windows(2) {
                    let ((_, before), (day, price)) = (pair[0], pair[1]);
                    total += part(price, day, w) - part(before, day, w);
                }
                total
            }
            Kind::Pack | Kind::BoughtPack => self
                .pack_charges(lineage, kind, win, &[])
                .iter()
                .map(|(_, _, amount)| amount)
                .sum(),
            Kind::Usage | Kind::OneTime => 0,
        }
    }

    /// The plan's renewals: the billing start, each plan window's grid boundaries while it is in
    /// force, and the ending's day.
    fn renewals(&self) -> Vec<NaiveDate> {
        let mut days = vec![self.start, self.end];
        for span in &self.spans {
            let from = span.from.unwrap_or(self.start).max(self.start);
            let to = span.to.unwrap_or(self.end).min(self.end);
            let Some(grid) = self.grid(span, span.cadence) else {
                continue;
            };
            let mut day = from;
            while day < to {
                if grid.is_boundary(day) {
                    days.push(day);
                }
                day = day + Days::new(1);
            }
        }
        days.sort();
        days.dedup();
        days
    }

    /// Each pack price of a window: its charge's period, the day its document is issued and what
    /// it bills. Never netted (R-103): a plan's pack bills its share from its day to the window's
    /// end, a bought pack its price whole, keyed on its day when it starts inside its period
    /// (R-075). One due at its period's start rides the next renewal (R-084), else its own day.
    fn pack_charges(
        &self,
        lineage: &Fee,
        kind: Kind,
        win: &Win,
        renewals: &[NaiveDate],
    ) -> Vec<(Period, NaiveDate, i64)> {
        let part = |price: i64, from: NaiveDate| {
            left(price, win.nominal, from) - left(price, win.nominal, win.until)
        };
        self.terms(lineage, win)
            .into_iter()
            .enumerate()
            .map(|(i, (day, price))| {
                let issued = if i == 0 && day == win.key.0 {
                    renewals.iter().copied().find(|r| *r >= day).unwrap_or(day)
                } else {
                    day
                };
                if kind == Kind::Pack {
                    return (win.key, issued, part(price, day));
                }
                let changed = i > 0 || (day != win.key.0 && day != self.start);
                (if changed { (day, day) } else { win.key }, issued, price)
            })
            .collect()
    }

    /// Every pack charge of the facts, by lineage.
    fn packs(&self) -> Vec<(Fee, Period, NaiveDate, i64)> {
        let mut lineages: Vec<Fee> = self.facts.fees.iter().map(|r| r.lineage.clone()).collect();
        lineages.sort();
        lineages.dedup();
        let mut out = Vec::new();
        let renewals = self.renewals();
        for lineage in lineages {
            let mut current: Option<Win> = None;
            for run in self.runs(&lineage) {
                let Some(row) = self.row(&lineage, run.from) else {
                    continue;
                };
                let kind = row.fee.kind;
                let windows = self.windows(&run, &mut current);
                if !matches!(kind, Kind::Pack | Kind::BoughtPack) {
                    continue;
                }
                for win in windows {
                    for (key, day, amount) in self.pack_charges(&lineage, kind, &win, &renewals) {
                        out.push((lineage.clone(), key, day, amount));
                    }
                }
            }
        }
        out
    }

    fn fixed(&self, lineage: &Fee) -> i64 {
        let mut current: Option<Win> = None;
        let mut total = 0;
        let mut once_seen = false;
        for run in self.runs(lineage) {
            let Some(row) = self.row(lineage, run.from) else {
                continue;
            };
            let kind = row.fee.kind;
            if run.billing == Billing::OneTime {
                // R-093: a plan's setup bills with the subscription's start only, never on a
                // later plan window; an add-on's one-time price bills on its day (R-075).
                let plan_setup = matches!(lineage, Fee::Component(_))
                    && run.from != self.start
                    && self.facts.plan_windows.iter().any(|w| w.from == run.from);
                if !once_seen && !plan_setup {
                    total += self.price(row, run.from);
                }
                once_seen = true;
                continue;
            }
            if kind == Kind::Usage {
                continue;
            }
            for win in self.windows(&run, &mut current) {
                total += self.window_amount(lineage, kind, &win);
            }
        }
        total
    }

    /// Units used while a usage row of the lineage was in force, from the billing start to the
    /// ending's instant (R-016), times that row's rate.
    fn usage(&self, lineage: &Fee, events: &Events) -> i64 {
        let ending = self.facts.effective_ending().expect("an ended story");
        let from = midnight(self.start);
        let mut total = 0;
        let mut day = self.start;
        while midnight(day) < ending.at {
            if let Some(row) = self.row(lineage, day)
                && row.fee.kind == Kind::Usage
            {
                let until = (midnight(day) + chrono::Duration::days(1)).min(ending.at);
                total += row.fee.amount * events.units(midnight(day).max(from), until);
            }
            day = day + Days::new(1);
        }
        total
    }
}

/// What each pack lineage bills over a story's life (R-103): reconcile never gives a pack back,
/// so each charge bills the most its facts owed at a pass, counting the documents issued by then.
pub fn packs_billed(built: &Built) -> BTreeMap<Fee, i64> {
    let last = Reference::new(built.last());
    let horizon = last.end + Days::new(400);
    let mut most: BTreeMap<(Fee, Period), i64> = BTreeMap::new();
    for (j, (at, facts)) in built.snapshots.iter().enumerate() {
        let next = built.snapshots.get(j + 1).map(|(next, _)| *next);
        if next == Some(*at) {
            continue;
        }
        let due = |day: NaiveDate| match next {
            Some(next) => midnight(day) < next,
            None => midnight(day) <= built.until,
        };
        let mut owed: BTreeMap<(Fee, Period), i64> = BTreeMap::new();
        for (lineage, key, day, amount) in Reference::until(facts, horizon).packs() {
            if due(day) {
                *owed.entry((lineage, key)).or_default() += amount;
            }
        }
        for (charge, amount) in owed {
            let entry = most.entry(charge).or_insert(amount);
            *entry = (*entry).max(amount);
        }
    }
    let mut by: BTreeMap<Fee, i64> = BTreeMap::new();
    for ((lineage, _), amount) in most {
        *by.entry(lineage).or_default() += amount;
    }
    by
}
