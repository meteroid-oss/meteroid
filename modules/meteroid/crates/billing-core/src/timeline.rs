//! What a subscription's fees bill over time, from its dated facts: the grid each fee bills on and
//! the windows it bills, its periods cut to its life (R-084).

use chrono::{Datelike, Months, NaiveDate};

/// Periods of `months` whose boundaries fall on the `anchor` day of the month, or the month's last
/// day when it is shorter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    origin: NaiveDate,
    first_end: NaiveDate,
    months: u32,
    anchor: u32,
    /// Whether the grid runs before its origin, in full periods.
    backwards: bool,
}

fn on_anchor(date: NaiveDate, anchor: u32) -> Option<NaiveDate> {
    let last = NaiveDate::from_ymd_opt(date.year(), date.month(), 1)?
        .checked_add_months(Months::new(1))?
        .pred_opt()?
        .day();
    date.with_day(last.min(anchor))
}

fn months_after(date: NaiveDate, months: u32, anchor: u32) -> Option<NaiveDate> {
    on_anchor(date.checked_add_months(Months::new(months))?, anchor)
}

fn months_before(date: NaiveDate, months: u32, anchor: u32) -> Option<NaiveDate> {
    on_anchor(date.checked_sub_months(Months::new(months))?, anchor)
}

impl Grid {
    /// The grid of a subscription billing from `start`: a first period up to the anchor day of
    /// the same month when `start` falls before it, else a full one; full periods after it.
    pub fn from_start(start: NaiveDate, anchor: u32, months: u32) -> Option<Self> {
        let first_end = if start.day() >= anchor {
            months_after(start, months, anchor)?
        } else {
            on_anchor(start, anchor)?
        };
        Some(Grid {
            origin: start,
            first_end,
            months: months.max(1),
            anchor,
            backwards: false,
        })
    }

    /// The grid of a subscription billing from the end of a free trial on `start`: its first period
    /// runs a full cadence on to the anchor day, then full periods (R-084).
    pub fn from_trial_end(start: NaiveDate, anchor: u32, months: u32) -> Option<Self> {
        Some(Grid {
            origin: start,
            first_end: months_after(start, months, anchor)?,
            months: months.max(1),
            anchor,
            backwards: false,
        })
    }

    /// A grid of full periods from `origin`, before it as after it: a cadence changed there.
    pub fn from_origin(origin: NaiveDate, anchor: u32, months: u32) -> Option<Self> {
        Some(Grid {
            origin,
            first_end: months_after(origin, months, anchor)?,
            months: months.max(1),
            anchor,
            backwards: true,
        })
    }

    pub fn months(&self) -> u32 {
        self.months
    }

    pub fn origin(&self) -> NaiveDate {
        self.origin
    }

    /// Whether it runs before its origin: a change of cadence's grid.
    pub fn runs_backwards(&self) -> bool {
        self.backwards
    }

    /// The `n`-th boundary: the origin for 0, before it for a negative `n` (a backwards grid).
    fn boundary(&self, n: i64) -> Option<NaiveDate> {
        match n {
            0 => Some(self.origin),
            n if n > 0 => {
                let after = u32::try_from(n - 1).ok()?.checked_mul(self.months)?;
                months_after(self.first_end, after, self.anchor)
            }
            n => {
                let before = u32::try_from(-n).ok()?.checked_mul(self.months)?;
                months_before(self.origin, before, self.anchor)
            }
        }
    }

    /// The index of the period holding `at`: the one starting at boundary `n`.
    fn index_of(&self, at: NaiveDate) -> Option<i64> {
        if at < self.origin && !self.backwards {
            return Some(0);
        }
        let months = i64::from(self.months);
        let elapsed = (i64::from(at.year()) - i64::from(self.first_end.year())) * 12
            + i64::from(at.month())
            - i64::from(self.first_end.month());
        let mut n = if at < self.first_end {
            if at >= self.origin {
                0
            } else {
                elapsed.div_euclid(months)
            }
        } else {
            elapsed.div_euclid(months) + 1
        };
        while self.boundary(n)? > at {
            n -= 1;
        }
        while self.boundary(n + 1)? <= at {
            n += 1;
        }
        Some(n)
    }

    /// The period `[start, end)` holding `at`; before a forward grid's origin, its first period.
    pub fn period_containing(&self, at: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
        let n = self.index_of(at)?;
        Some((self.boundary(n)?, self.boundary(n + 1)?))
    }

    /// The full period `period` is priced over: a grid's first period when it is a stub ends
    /// where a full one from the anchor would, else itself.
    pub fn nominal(&self, period: (NaiveDate, NaiveDate)) -> (NaiveDate, NaiveDate) {
        if !self.backwards
            && period == (self.origin, self.first_end)
            && let Some(start) = months_before(self.first_end, self.months, self.anchor)
            && start < self.origin
        {
            return (start, self.first_end);
        }
        period
    }

    /// Whether a period starts on `at`.
    pub fn is_boundary(&self, at: NaiveDate) -> bool {
        self.period_containing(at)
            .is_some_and(|(start, _)| start == at)
            && (self.backwards || at >= self.origin)
    }
}

/// A change of the subscription's cadence, from `before` to `after` months, on `on`; `recorded`
/// is the origin an earlier release's renewals fixed, when one was recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CadenceSwitch {
    pub on: NaiveDate,
    pub before: u32,
    pub after: u32,
    pub recorded: Option<NaiveDate>,
}

/// Where each change of cadence's grid starts, earliest first: its recorded origin, else its first
/// renewal, the change's day on a boundary of the grid in force before it, else the first boundary
/// after it of the new cadence's grid through that grid's next boundary.
pub fn cadence_origins(
    billing_start: NaiveDate,
    anchor: u32,
    switches: &[CadenceSwitch],
) -> Vec<(NaiveDate, NaiveDate)> {
    let mut origins = Vec::new();
    let mut previous: Option<(NaiveDate, u32)> = None;
    for switch in switches {
        let grid = match previous {
            Some((origin, months)) => Grid::from_origin(origin, anchor, months),
            None => Grid::from_start(billing_start, anchor, switch.before),
        };
        let origin = switch.recorded.or_else(|| {
            let grid = grid?;
            if grid.is_boundary(switch.on) {
                return Some(switch.on);
            }
            let (_, next) = grid.period_containing(switch.on)?;
            // A shorter cadence renews before the old period ends.
            Grid::from_origin(next, anchor, switch.after)?
                .period_containing(switch.on)
                .map(|(_, renewal)| renewal)
        });
        if let Some(origin) = origin {
            origins.push((switch.on, origin));
            previous = Some((origin, switch.after));
        }
    }
    origins
}

/// A part of a grid period a fee lives through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub period: (NaiveDate, NaiveDate),
    pub from: NaiveDate,
    pub until: NaiveDate,
}

impl Window {
    /// Whether it covers its whole period.
    pub fn is_whole(&self) -> bool {
        (self.from, self.until) == self.period
    }
}

/// The windows of a fee living through `[from, to)` on `grid`, up to `through`: each period it
/// lives in, cut to its life. A forward grid starts at its origin, whatever the life.
pub fn windows(
    grid: &Grid,
    (from, to): (NaiveDate, Option<NaiveDate>),
    through: NaiveDate,
) -> Vec<Window> {
    let from = if grid.backwards {
        from
    } else {
        from.max(grid.origin)
    };
    let end = to.map_or(through, |to| to.min(through));
    let mut windows = Vec::new();
    let mut cursor = from;
    while cursor < end {
        let Some(period) = grid.period_containing(cursor) else {
            break;
        };
        let until = period.1.min(end);
        windows.push(Window {
            period,
            from: cursor,
            until,
        });
        cursor = until;
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn a_start_before_the_anchor_day_bills_a_first_period_up_to_it() {
        let grid = Grid::from_start(d(2024, 1, 10), 15, 1).unwrap();
        assert_eq!(
            grid.period_containing(d(2024, 1, 12)),
            Some((d(2024, 1, 10), d(2024, 1, 15)))
        );
        assert_eq!(
            grid.period_containing(d(2024, 1, 15)),
            Some((d(2024, 1, 15), d(2024, 2, 15)))
        );
        assert_eq!(
            grid.period_containing(d(2024, 3, 14)),
            Some((d(2024, 2, 15), d(2024, 3, 15)))
        );
    }

    /// Anchored on the 31st: February ends on its last day, March on the 31st again.
    #[test]
    fn a_month_end_anchor_falls_on_each_months_last_day() {
        let grid = Grid::from_start(d(2024, 1, 31), 31, 1).unwrap();
        assert_eq!(
            grid.period_containing(d(2024, 2, 10)),
            Some((d(2024, 1, 31), d(2024, 2, 29)))
        );
        assert_eq!(
            grid.period_containing(d(2024, 2, 29)),
            Some((d(2024, 2, 29), d(2024, 3, 31)))
        );
        assert_eq!(
            grid.period_containing(d(2025, 3, 1)),
            Some((d(2025, 2, 28), d(2025, 3, 31)))
        );
    }

    #[test]
    fn a_yearly_grid_from_a_leap_day_renews_on_the_last_day_of_february() {
        let grid = Grid::from_start(d(2024, 2, 29), 29, 12).unwrap();
        assert_eq!(
            grid.period_containing(d(2025, 3, 1)),
            Some((d(2025, 2, 28), d(2026, 2, 28)))
        );
        assert_eq!(
            grid.period_containing(d(2028, 2, 29)),
            Some((d(2028, 2, 29), d(2029, 2, 28)))
        );
    }

    /// A quarterly cadence switched to on April 15 runs from there, and its period before the
    /// origin is the full quarter ending on it.
    #[test]
    fn a_grid_from_a_change_of_cadence_runs_backwards_in_full_periods() {
        let grid = Grid::from_origin(d(2024, 4, 15), 15, 3).unwrap();
        assert_eq!(
            grid.period_containing(d(2024, 4, 1)),
            Some((d(2024, 1, 15), d(2024, 4, 15)))
        );
        assert_eq!(
            grid.period_containing(d(2024, 8, 1)),
            Some((d(2024, 7, 15), d(2024, 10, 15)))
        );
        assert!(grid.is_boundary(d(2023, 10, 15)));
        assert!(!grid.is_boundary(d(2024, 5, 15)));
    }

    #[test]
    fn a_first_stub_is_priced_over_the_full_period_ending_with_it() {
        let grid = Grid::from_start(d(2024, 1, 10), 15, 3).unwrap();
        assert_eq!(
            grid.nominal((d(2024, 1, 10), d(2024, 1, 15))),
            (d(2023, 10, 15), d(2024, 1, 15))
        );
        assert_eq!(
            grid.nominal((d(2024, 1, 15), d(2024, 4, 15))),
            (d(2024, 1, 15), d(2024, 4, 15))
        );
        let full = Grid::from_start(d(2024, 1, 15), 15, 1).unwrap();
        assert_eq!(
            full.nominal((d(2024, 1, 15), d(2024, 2, 15))),
            (d(2024, 1, 15), d(2024, 2, 15))
        );
    }

    /// A free trial ending on February 3 of a subscription renewing on the 20th: the first period
    /// runs to March 20 unprorated; one ending February 25 runs to March 20 as 23 of 28 days.
    #[test]
    fn after_a_free_trial_the_first_period_runs_a_full_cadence_on_to_the_anchor() {
        let early = Grid::from_trial_end(d(2024, 2, 3), 20, 1).unwrap();
        assert_eq!(
            early.period_containing(d(2024, 2, 3)),
            Some((d(2024, 2, 3), d(2024, 3, 20)))
        );
        assert_eq!(
            early.nominal((d(2024, 2, 3), d(2024, 3, 20))),
            (d(2024, 2, 3), d(2024, 3, 20))
        );
        assert_eq!(
            early.period_containing(d(2024, 3, 20)),
            Some((d(2024, 3, 20), d(2024, 4, 20)))
        );
        let late = Grid::from_trial_end(d(2024, 2, 25), 20, 1).unwrap();
        assert_eq!(
            late.nominal((d(2024, 2, 25), d(2024, 3, 20))),
            (d(2024, 2, 20), d(2024, 3, 20))
        );
    }

    /// A fee added on the 10th and removed on the 20th of the second month lives through a part
    /// of each period.
    #[test]
    fn a_fee_bills_each_period_it_lives_in_cut_to_its_life() {
        let grid = Grid::from_start(d(2024, 1, 1), 1, 1).unwrap();
        let windows = windows(
            &grid,
            (d(2024, 1, 10), Some(d(2024, 2, 20))),
            d(2024, 12, 31),
        );
        assert_eq!(
            windows,
            vec![
                Window {
                    period: (d(2024, 1, 1), d(2024, 2, 1)),
                    from: d(2024, 1, 10),
                    until: d(2024, 2, 1)
                },
                Window {
                    period: (d(2024, 2, 1), d(2024, 3, 1)),
                    from: d(2024, 2, 1),
                    until: d(2024, 2, 20)
                },
            ]
        );
    }

    fn switch(on: NaiveDate, before: u32, after: u32) -> CadenceSwitch {
        CadenceSwitch {
            on,
            before,
            after,
            recorded: None,
        }
    }

    /// A change of cadence's grid starts on its first renewal: a yearly subscription switched to
    /// months or quarters on Mar 15 renews on Apr 1, not at the year's end; a monthly one switched
    /// to a year on Jan 15 renews on Feb 1, and back to months on May 10, on Jun 1.
    #[test]
    fn a_change_of_cadence_runs_from_its_first_renewal() {
        for months in [1, 3] {
            assert_eq!(
                cadence_origins(d(2024, 1, 1), 1, &[switch(d(2024, 3, 15), 12, months)]),
                vec![(d(2024, 3, 15), d(2024, 4, 1))]
            );
        }
        assert_eq!(
            cadence_origins(
                d(2024, 1, 1),
                1,
                &[switch(d(2024, 1, 15), 1, 12), switch(d(2024, 5, 10), 12, 1)]
            ),
            vec![
                (d(2024, 1, 15), d(2024, 2, 1)),
                (d(2024, 5, 10), d(2024, 6, 1))
            ]
        );
        assert_eq!(
            cadence_origins(d(2024, 1, 1), 1, &[switch(d(2025, 1, 1), 12, 1)]),
            vec![(d(2025, 1, 1), d(2025, 1, 1))]
        );
    }

    proptest::proptest! {
        /// The periods of any grid tile time: each starts where the previous ended.
        #[test]
        fn periods_tile_time(
            start in 0u32..3000, anchor in 1u32..32, months in proptest::sample::select(vec![1u32, 3, 6, 12]),
            backwards in proptest::bool::ANY,
        ) {
            let start = d(2020, 1, 1) + chrono::Days::new(u64::from(start));
            let grid = if backwards {
                Grid::from_origin(start, anchor, months).unwrap()
            } else {
                Grid::from_start(start, anchor, months).unwrap()
            };
            let mut at = if backwards { start - chrono::Days::new(400) } else { start };
            let (_, mut previous_end) = grid.period_containing(at).unwrap();
            for _ in 0..40 {
                at = previous_end;
                let (period_start, period_end) = grid.period_containing(at).unwrap();
                proptest::prop_assert_eq!(period_start, previous_end);
                proptest::prop_assert!(period_end > period_start);
                previous_end = period_end;
            }
        }
    }
}
