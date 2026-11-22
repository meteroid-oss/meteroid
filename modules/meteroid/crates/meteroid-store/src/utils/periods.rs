use common_utils::date::NaiveDateExt;

use crate::domain::enums::BillingPeriodEnum;
use crate::domain::{ComponentPeriods, Period};
use crate::services::proration::DayFraction;
use chrono::{Datelike, Months, NaiveDate};

/**
 * For a given invoice date, this checks if a component with this period should be billed in that invoice and what are the related arrear/advance periods
 */
/// Factor = partial_days / full_days in whole days, below one. Returns `None` only when no
/// proration is needed (partial covers the whole reference period) or inputs are invalid.
pub fn calculate_proration_factor(partial: &Period, full: &Period) -> Option<DayFraction> {
    let partial_days = partial.end.signed_duration_since(partial.start).num_days();
    let full_days = full.end.signed_duration_since(full.start).num_days();

    if full_days <= 0 || partial_days < 0 || partial_days >= full_days {
        return None;
    }

    Some(DayFraction::new(partial_days, full_days))
}

pub fn calculate_advance_period_range(
    invoice_date: NaiveDate,
    billing_day: u32,
    is_partial: bool,
    billing_period: &BillingPeriodEnum,
) -> Period {
    let months_per_period = billing_period.as_months();

    // Check if billing started on or after the target billing day
    let started_after_billing_day = invoice_date.day() >= billing_day;

    let period_start = invoice_date;

    let period_end = {
        let should_add_full_period = started_after_billing_day || !is_partial;

        if should_add_full_period {
            add_months_at_billing_day(period_start, months_per_period, billing_day)
                .expect("Failed to calculate period end date")
        } else {
            // For the first period when billing started before the billing day,
            // end on the billing day of the current month
            let target_day = period_start.days_in_month().min(billing_day);
            period_start
                .with_day(target_day)
                .expect("Failed to set period end day")
        }
    };

    Period {
        start: period_start,
        end: period_end,
    }
}

/// Calculate the number of complete billing cycles between billing_start_date and target_date.
/// A cycle is complete when target_date is on or after the cycle end date.
pub fn calculate_elapsed_cycles(
    billing_start_date: NaiveDate,
    target_date: NaiveDate,
    billing_period: &BillingPeriodEnum,
    billing_day_anchor: u32,
) -> u32 {
    if target_date <= billing_start_date {
        return 0;
    }

    let first_period = calculate_advance_period_range(
        billing_start_date,
        billing_day_anchor,
        true,
        billing_period,
    );

    // target is within the first (possibly partial) period
    if target_date < first_period.end {
        return 0;
    }

    // first_period.end is where full-length cycles begin
    let first_full_start = first_period.end;
    let full_cycles_since_first = count_full_periods_up_to(
        first_full_start,
        target_date,
        billing_day_anchor,
        billing_period,
    );

    // +1 accounts for the first (possibly partial) period that completed at first_full_start
    full_cycles_since_first + 1
}

/// Find the billing period that contains the given target_date.
/// Returns the period where period.start <= target_date < period.end.
pub fn find_period_containing_date(
    billing_start_date: NaiveDate,
    target_date: NaiveDate,
    billing_period: &BillingPeriodEnum,
    billing_day_anchor: u32,
) -> Period {
    if target_date < billing_start_date {
        return calculate_advance_period_range(
            billing_start_date,
            billing_day_anchor,
            true,
            billing_period,
        );
    }

    let first_period = calculate_advance_period_range(
        billing_start_date,
        billing_day_anchor,
        true,
        billing_period,
    );

    // target is within the first (possibly partial) period
    if target_date < first_period.end {
        return first_period;
    }

    let first_full_start = first_period.end;
    let cycle_index = find_period_index_at(
        first_full_start,
        target_date,
        billing_day_anchor,
        billing_period,
    );

    let period_start = add_months_at_billing_day(
        first_full_start,
        cycle_index * billing_period.as_months(),
        billing_day_anchor,
    )
    .expect("Failed to calculate period start");

    calculate_advance_period_range(period_start, billing_day_anchor, false, billing_period)
}

/// O(1) computation of how many complete periods fit in [period_origin, target_date],
/// where a period is complete when its end <= target_date.
fn count_full_periods_up_to(
    period_origin: NaiveDate,
    target_date: NaiveDate,
    billing_day: u32,
    billing_period: &BillingPeriodEnum,
) -> u32 {
    let months_per_period = billing_period.as_months();
    let total_months = (target_date.year() - period_origin.year()) * 12
        + (target_date.month() as i32 - period_origin.month() as i32);

    if total_months <= 0 {
        // target is in the same month or earlier — check if it's on the period boundary exactly
        if target_date
            >= add_months_at_billing_day(period_origin, months_per_period, billing_day)
                .expect("Failed to calculate period end")
        {
            return 1;
        }
        return 0;
    }

    // Candidate: how many full billing periods fit in total_months
    let candidate = total_months as u32 / months_per_period;

    // The end of the candidate-th period is the start of the (candidate+1)-th period
    let candidate_end = add_months_at_billing_day(
        period_origin,
        (candidate + 1) * months_per_period,
        billing_day,
    )
    .expect("Failed to calculate candidate period end");

    if candidate_end <= target_date {
        // We can fit one more complete period
        candidate + 1
    } else {
        // Check if the (candidate-1)-th period's end (= candidate-th period's start) is <= target
        let prev_period_end =
            add_months_at_billing_day(period_origin, candidate * months_per_period, billing_day)
                .expect("Failed to calculate period boundary");

        if prev_period_end <= target_date {
            candidate
        } else {
            candidate.saturating_sub(1)
        }
    }
}

/// O(1) computation of which period index (0-based from period_origin) contains target_date.
fn find_period_index_at(
    period_origin: NaiveDate,
    target_date: NaiveDate,
    billing_day: u32,
    billing_period: &BillingPeriodEnum,
) -> u32 {
    let months_per_period = billing_period.as_months();
    let total_months = (target_date.year() - period_origin.year()) * 12
        + (target_date.month() as i32 - period_origin.month() as i32);

    let candidate = if total_months <= 0 {
        0
    } else {
        total_months as u32 / months_per_period
    };

    // Compute where the candidate period starts and verify target_date is within it
    let candidate_start =
        add_months_at_billing_day(period_origin, candidate * months_per_period, billing_day)
            .expect("Failed to calculate candidate period start");

    if target_date < candidate_start {
        // Day-of-month edge case: we overestimated by one period
        candidate.saturating_sub(1)
    } else {
        // Check we haven't underestimated: if target is past the candidate period's end,
        // advance by one
        let candidate_end = add_months_at_billing_day(
            period_origin,
            (candidate + 1) * months_per_period,
            billing_day,
        )
        .expect("Failed to calculate candidate period end");

        if target_date >= candidate_end {
            candidate + 1
        } else {
            candidate
        }
    }
}

pub fn calculate_arrear_period_range(
    invoice_date: NaiveDate,
    billing_start_or_resume_date: NaiveDate,
    billing_day: u32,
    billing_period: &BillingPeriodEnum,
) -> Period {
    // Arrear bills the period already ended at `invoice_date`.
    let period_end = invoice_date;

    // Resolve the start from the period containing the day before `invoice_date`: correct for both
    // boundary dates (renewals, end-of-period cancels) and mid-period dates (immediate cancel),
    // unlike a fixed month-subtract which overshoots a whole period back on a mid-period date.
    let period_start = if period_end <= billing_start_or_resume_date {
        billing_start_or_resume_date
    } else {
        let day_before_end = period_end.pred_opt().unwrap_or(period_end);
        find_period_containing_date(
            billing_start_or_resume_date,
            day_before_end,
            billing_period,
            billing_day,
        )
        .start
        .max(billing_start_or_resume_date)
    };

    Period {
        start: period_start,
        end: period_end,
    }
}

fn add_months_at_billing_day(
    date: NaiveDate,
    months_to_add: u32,
    billing_day: u32,
) -> Option<NaiveDate> {
    date.checked_add_months(Months::new(months_to_add))
        .and_then(|d| d.with_day(d.days_in_month().min(billing_day)))
}

pub(crate) fn subtract_months_at_billing_day(
    date: NaiveDate,
    months_to_subtract: u32,
    billing_day: u32,
) -> Option<NaiveDate> {
    date.checked_sub_months(Months::new(months_to_subtract))
        .and_then(|d| d.with_day(d.days_in_month().min(billing_day)))
}

/// The change of cadence whose grid a record living over `[from, to)` bills on (`runs`, latest
/// first): the latest on or before its start, as a record kept through a change keeps its grid
/// (R-084); one with no start, the latest before it ended. None: the billing start's grid.
pub fn run_in_force(
    runs: &[CadenceRun],
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> Option<&CadenceRun> {
    match from {
        Some(from) => runs.iter().find(|run| run.switched_on <= from),
        None => runs
            .iter()
            .find(|run| to.is_none_or(|to| to > run.switched_on)),
    }
}

/// Periods after a change of cadence: they run from `origin` in full lengths of the cadence,
/// whatever the cadence before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CadenceRun {
    /// The plan switch that changed the cadence.
    pub switched_on: NaiveDate,
    /// Where the new grid starts: the first renewal on or after the switch, or the one after it
    /// when an earlier release renewed the new cadence from there.
    pub origin: NaiveDate,
    /// The latest renewal before the invoice being computed.
    pub previous_renewal: NaiveDate,
}

impl CadenceRun {
    /// The `n`-th boundary of a `months` grid from the origin.
    pub fn boundary(&self, n: u32, months: u32, billing_day: u32) -> Option<NaiveDate> {
        add_months_at_billing_day(self.origin, n.checked_mul(months)?, billing_day)
    }

    /// The grid period `[start, end)` holding `date`, `None` before the origin.
    pub fn period_holding(&self, date: NaiveDate, months: u32, billing_day: u32) -> Option<Period> {
        if date < self.origin || months == 0 {
            return None;
        }
        let mut n = 0;
        loop {
            let start = self.boundary(n, months, billing_day)?;
            let end = self.boundary(n + 1, months, billing_day)?;
            if date < end {
                return Some(Period { start, end });
            }
            n += 1;
        }
    }

    /// Whether `date` is a boundary of a `months` grid from the origin.
    pub fn is_boundary(&self, date: NaiveDate, months: u32, billing_day: u32) -> bool {
        self.period_holding(date, months, billing_day)
            .is_some_and(|period| period.start == date)
    }
}

/// The grid a subscription's fees and renewals run on before any change of cadence (R-084): from
/// the billing start, or from the end of a free trial (`billing_from` after it).
pub fn base_grid(
    billing_start: NaiveDate,
    billing_from: NaiveDate,
    billing_day: u32,
    months: u32,
) -> Option<billing_core::timeline::Grid> {
    if billing_from > billing_start {
        billing_core::timeline::Grid::from_trial_end(billing_from, billing_day, months)
    } else {
        billing_core::timeline::Grid::from_start(billing_start, billing_day, months)
    }
}

/// The period of `grid` holding `at`, and how many of its periods ended before it.
pub fn cycle_on(grid: &billing_core::timeline::Grid, at: NaiveDate) -> Option<(Period, u32)> {
    let mut elapsed = 0;
    let mut cursor = grid.origin();
    loop {
        let (start, end) = grid.period_containing(cursor)?;
        if at < end {
            return Some((Period { start, end }, elapsed));
        }
        elapsed += 1;
        cursor = end;
    }
}

/// A fee's grid as an invoice reads it (R-084): where the fee started on it, and the renewal before
/// the invoice, since which what started is billed.
#[derive(Clone, Copy, Debug)]
pub struct FeeRun {
    pub grid: billing_core::timeline::Grid,
    pub switched_on: NaiveDate,
    pub previous_renewal: NaiveDate,
}

impl FeeRun {
    /// A fee on a change of cadence's grid.
    pub fn of_cadence_run(run: &CadenceRun, months: u32, billing_day: u32) -> Option<Self> {
        Some(FeeRun {
            grid: billing_core::timeline::Grid::from_origin(run.origin, billing_day, months)?,
            switched_on: run.switched_on,
            previous_renewal: run.previous_renewal,
        })
    }

    /// A fee on the base grid of its own cadence (see [`base_grid`]), renewed on it.
    pub fn from_start(
        (billing_start, billing_from): (NaiveDate, NaiveDate),
        billing_day: u32,
        months: u32,
        invoice_date: NaiveDate,
    ) -> Option<Self> {
        let grid = base_grid(billing_start, billing_from, billing_day, months)?;
        let origin = grid.origin();
        let previous_renewal = match invoice_date.pred_opt() {
            Some(day) if invoice_date > origin => grid.period_containing(day)?.0,
            _ => origin,
        };
        Some(FeeRun {
            grid,
            switched_on: origin,
            previous_renewal,
        })
    }

    fn period(&self, at: NaiveDate) -> Option<Period> {
        let (start, end) = self.grid.period_containing(at)?;
        Some(Period { start, end })
    }

    fn nominal(&self, period: &Period) -> Period {
        let (start, end) = self.grid.nominal((period.start, period.end));
        Period { start, end }
    }

    /// What the invoice of `invoice_date` settles in arrears, one window per grid period with the
    /// period it is priced over (see [`CadenceRun::arrear_windows`]).
    pub(crate) fn arrear_windows(
        &self,
        invoice_date: NaiveDate,
        settles_early: bool,
    ) -> Vec<(Period, Period)> {
        let origin = self.grid.origin();
        if invoice_date <= origin.min(self.previous_renewal) {
            return vec![];
        }
        let Some(last) = invoice_date.pred_opt().and_then(|day| self.period(day)) else {
            return vec![];
        };
        if invoice_date == origin && self.grid.runs_backwards() {
            let start = last.start.max(self.previous_renewal);
            if start >= invoice_date {
                return vec![];
            }
            return vec![(
                Period {
                    start,
                    end: invoice_date,
                },
                last,
            )];
        }
        if !settles_early && !self.grid.is_boundary(invoice_date) {
            return vec![];
        }
        let mut cursor = self.previous_renewal.min(last.start).max(origin);
        let mut windows = Vec::new();
        while cursor < invoice_date {
            let Some(grid) = self.period(cursor) else {
                break;
            };
            let end = grid.end.min(invoice_date);
            windows.push((Period { start: cursor, end }, self.nominal(&grid)));
            cursor = end;
        }
        windows
    }

    /// [`Self::arrear_windows`] of a fee: one whose grid the renewals are off (a row kept through a
    /// change of cadence) settles from the start of its grid period, as arrears settle only on
    /// its boundaries (R-084).
    fn fee_arrear_windows(
        &self,
        invoice_date: NaiveDate,
        settles_early: bool,
    ) -> Vec<(Period, Period)> {
        let mut windows = self.arrear_windows(invoice_date, settles_early);
        let settles_the_old_grid = invoice_date == self.grid.origin() && self.grid.runs_backwards();
        if !settles_the_old_grid
            && let Some((first, _)) = windows.first_mut()
            && !self.grid.is_boundary(first.start)
            && let Some(period) = self.period(first.start)
        {
            first.start = period.start;
        }
        windows
    }

    /// Whether a fee ending on `to`, off its grid, settles on the invoice of `invoice_date`: one
    /// from its end while its last period runs, as no boundary of its own settles it (R-011).
    pub fn ends_off_grid(&self, to: NaiveDate, invoice_date: NaiveDate) -> bool {
        to <= invoice_date
            && !self.grid.is_boundary(to)
            && to
                .pred_opt()
                .and_then(|day| self.period(day))
                .is_some_and(|last| invoice_date <= last.end)
    }

    /// The window a fee ending on `to` off its grid settles: its last period up to `to`, with the
    /// period it is priced over.
    fn ended_window(&self, to: NaiveDate) -> Option<(Period, Period)> {
        let last = self.period(to.pred_opt()?)?;
        Some((
            Period {
                start: last.start,
                end: to,
            },
            self.nominal(&last),
        ))
    }

    /// The advance periods the invoice of `invoice_date` bills: the one starting on it, and any
    /// started since the previous renewal that no renewal billed, none before `not_before`.
    fn advance_periods(&self, invoice_date: NaiveDate, not_before: NaiveDate) -> Vec<Period> {
        let mut periods = Vec::new();
        let mut cursor = self.grid.origin().max(not_before);
        while let Some(period) = self.period(cursor) {
            if period.start > invoice_date {
                break;
            }
            if period.start >= cursor
                && (period.start > self.previous_renewal || period.start == invoice_date)
            {
                periods.push(period.clone());
            }
            cursor = period.end;
        }
        periods
    }
}

/// The periods a fee bills on the invoice of `invoice_date` on its grid: its arrears cut to its life
/// (its last window when it `settles`, see [`FeeRun::ends_off_grid`]), priced over full periods, and
/// each advance period, a first stub priced over the full period ending with it, cut at `term_end`.
pub fn component_periods_on(
    run: &FeeRun,
    invoice_date: NaiveDate,
    effective_from: Option<NaiveDate>,
    effective_to: Option<NaiveDate>,
    term_end: Option<NaiveDate>,
    is_completed: bool,
    settles: bool,
) -> Vec<ComponentPeriods> {
    let mut periods: Vec<ComponentPeriods> = Vec::new();
    // Ended the day it started billing: usage up to the termination instant still settles.
    if is_completed
        && !run.grid.runs_backwards()
        && invoice_date == run.grid.origin()
        && effective_from.is_none_or(|from| from <= invoice_date)
    {
        periods.push(ComponentPeriods {
            proration_factor: None,
            proration_days: None,
            arrear_proration_factor: Some(DayFraction::new(0, 1)),
            arrear_share: Some(DayFraction::new(0, 1)),
            advance: None,
            arrear: Some(Period {
                start: invoice_date,
                end: invoice_date,
            }),
        });
    }
    let arrears = match effective_to
        .filter(|_| settles && !is_completed)
        .and_then(|to| run.ended_window(to))
    {
        Some(ended) => vec![ended],
        None => run.fee_arrear_windows(invoice_date, is_completed),
    };
    for (window, nominal) in arrears {
        let start = effective_from.map_or(window.start, |from| from.max(window.start));
        let end = effective_to.map_or(window.end, |to| to.min(window.end));
        if start >= end {
            continue;
        }
        let days = (end - start).num_days();
        let nominal_days = (nominal.end - nominal.start).num_days();
        // A fixed term's end leaves an arrears amount whole (R-002).
        let by_term = term_end == Some(end) && effective_to.is_none_or(|to| to > end);
        let cut = end < nominal.end || start > nominal.start;
        periods.push(ComponentPeriods {
            proration_factor: None,
            proration_days: None,
            arrear_proration_factor: (days < nominal_days && !by_term)
                .then(|| DayFraction::new(days, nominal_days)),
            arrear_share: (cut && days < nominal_days)
                .then(|| DayFraction::new(days, nominal_days)),
            advance: None,
            arrear: Some(Period { start, end }),
        });
    }
    let not_before = effective_from.unwrap_or(run.switched_on);
    for period in run.advance_periods(invoice_date, not_before) {
        // Ended on the invoice's day, a period begun since the last renewal bills up to it.
        if is_completed && period.start >= invoice_date {
            continue;
        }
        let nominal = run.nominal(&period);
        let end = if is_completed {
            period.end.min(invoice_date)
        } else {
            period.end
        };
        let advance = Period {
            start: period.start,
            end: term_end.map_or(end, |term| term.min(end)),
        };
        if advance.start >= advance.end {
            continue;
        }
        let proration_factor = calculate_proration_factor(&advance, &nominal);
        periods.push(ComponentPeriods {
            proration_days: proration_factor.map(|_| {
                (
                    (advance.end - advance.start).num_days(),
                    (nominal.end - nominal.start).num_days(),
                )
            }),
            proration_factor,
            arrear_proration_factor: None,
            arrear_share: None,
            advance: Some(advance),
            arrear: None,
        });
    }
    periods
}

#[cfg(test)]
mod test {
    use super::{
        Period, calculate_advance_period_range, calculate_arrear_period_range,
        calculate_elapsed_cycles, calculate_proration_factor, find_period_containing_date,
    };
    use crate::domain::enums::BillingPeriodEnum;

    use chrono::NaiveDate;

    #[cfg(test)]
    mod tests {
        use super::*;
        use rstest::rstest;

        // Tests for advance billing (billing for upcoming periods)
        #[rstest]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-01-01", // invoice_date (start of first period)
            1,
            true,  // is_first_period
            "2021-01-01",
            "2021-02-01"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-02-01", // invoice_date (start of second period)
            1,
            false, // not first period
            "2021-02-01",
            "2021-03-01"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-03-01", // invoice_date (start of third period)
            1,
            false,
            "2021-03-01",
            "2021-04-01"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-01-10", // invoice_date
            1,
            true,  // first period
            "2021-01-10",
            "2021-02-01"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-02-01", // invoice_date
            1,
            false,
            "2021-02-01",
            "2021-03-01"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-03-01", // invoice_date
            1,
            false,
            "2021-03-01",
            "2021-04-01"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-01-01", // invoice_date
            10,
            true,  // first period
            "2021-01-01",
            "2021-01-10"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-01-10", // invoice_date
            10,
            false,
            "2021-01-10",
            "2021-02-10"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-02-10", // invoice_date
            10,
            false,
            "2021-02-10",
            "2021-03-10"
        )]
        #[case(
            BillingPeriodEnum::Quarterly,
            "2021-01-10", // invoice_date
            1,
            true,
            "2021-01-10",
            "2021-04-01"
        )]
        #[case(
            BillingPeriodEnum::Quarterly,
            "2021-04-01", // invoice_date
            1,
            false,
            "2021-04-01",
            "2021-07-01"
        )]
        #[case(
            BillingPeriodEnum::Quarterly,
            "2021-07-01", // invoice_date
            1,
            false,
            "2021-07-01",
            "2021-10-01"
        )]
        #[case(
            BillingPeriodEnum::Quarterly,
            "2021-01-01", // invoice_date
            10,
            true,
            "2021-01-01",
            "2021-01-10"
        )]
        #[case(
            BillingPeriodEnum::Quarterly,
            "2021-01-10", // invoice_date
            10,
            false,
            "2021-01-10",
            "2021-04-10"
        )]
        #[case(
            BillingPeriodEnum::Quarterly,
            "2021-04-10", // invoice_date
            10,
            false,
            "2021-04-10",
            "2021-07-10"
        )]
        #[case(
            BillingPeriodEnum::Annual,
            "2021-01-10", // invoice_date
            1,
            true,
            "2021-01-10",
            "2022-01-01"
        )]
        #[case(
            BillingPeriodEnum::Annual,
            "2022-01-01", // invoice_date
            1,
            false,
            "2022-01-01",
            "2023-01-01"
        )]
        #[case(
            BillingPeriodEnum::Annual,
            "2023-01-01", // invoice_date
            1,
            false,
            "2023-01-01",
            "2024-01-01"
        )]
        fn test_calculate_advance_period_range(
            #[case] billing_period: BillingPeriodEnum,
            #[case] invoice_date: NaiveDate,
            #[case] billing_day: u32,
            #[case] is_first_period: bool,
            #[case] expected_period_start: NaiveDate,
            #[case] expected_period_end: NaiveDate,
        ) {
            let Period { start, end } = calculate_advance_period_range(
                invoice_date,
                billing_day,
                is_first_period,
                &billing_period,
            );
            assert_eq!(start, expected_period_start);
            assert_eq!(end, expected_period_end);
        }

        // Tests for arrear billing (billing for completed periods)
        #[rstest]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-02-01", // invoice_date (end of period)
            "2021-01-01", // billing_start_date
            1,
            "2021-01-01", // max(calculated_start, billing_start) = max(2021-01-01, 2021-01-01)
            "2021-02-01"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-03-01", // invoice_date
            "2021-01-01", // billing_start_date
            1,
            "2021-02-01", // calculated back 1 month from 2021-03-01
            "2021-03-01"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-04-01", // invoice_date
            "2021-01-01", // billing_start_date
            1,
            "2021-03-01", // calculated back 1 month from 2021-04-01
            "2021-04-01"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-02-01", // invoice_date
            "2021-01-10", // billing_start_date (service started mid-month)
            1,
            "2021-01-10", // max(2021-01-01, 2021-01-10) = 2021-01-10
            "2021-02-01"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-01-10", // invoice_date
            "2021-01-01", // billing_start_date
            10,
            "2021-01-01", // max(2020-12-10, 2021-01-01) = 2021-01-01
            "2021-01-10"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-02-10", // invoice_date
            "2021-01-01", // billing_start_date
            10,
            "2021-01-10", // calculated back 1 month from 2021-02-10
            "2021-02-10"
        )]
        #[case(
            BillingPeriodEnum::Quarterly,
            "2021-04-01", // invoice_date
            "2021-01-10", // billing_start_date
            1,
            "2021-01-10", // max(2021-01-01, 2021-01-10) = 2021-01-10
            "2021-04-01"
        )]
        #[case(
            BillingPeriodEnum::Quarterly,
            "2021-07-01", // invoice_date
            "2021-01-10", // billing_start_date
            1,
            "2021-04-01", // calculated back 3 months from 2021-07-01
            "2021-07-01"
        )]
        #[case(
            BillingPeriodEnum::Annual,
            "2022-01-01", // invoice_date
            "2021-01-10", // billing_start_date
            1,
            "2021-01-10", // max(2021-01-01, 2021-01-10) = 2021-01-10
            "2022-01-01"
        )]
        #[case(
            BillingPeriodEnum::Annual,
            "2023-01-01", // invoice_date
            "2021-01-10", // billing_start_date
            1,
            "2022-01-01", // calculated back 12 months from 2023-01-01
            "2023-01-01"
        )]
        fn test_calculate_arrear_period_range(
            #[case] billing_period: BillingPeriodEnum,
            #[case] invoice_date: NaiveDate,
            #[case] billing_start_or_resume_date: NaiveDate,
            #[case] billing_day: u32,
            #[case] expected_period_start: NaiveDate,
            #[case] expected_period_end: NaiveDate,
        ) {
            let Period { start, end } = calculate_arrear_period_range(
                invoice_date,
                billing_start_or_resume_date,
                billing_day,
                &billing_period,
            );
            assert_eq!(start, expected_period_start);
            assert_eq!(end, expected_period_end);
        }

        // ─── Tests for calculate_elapsed_cycles ───

        #[rstest]
        // Same day → 0 elapsed
        #[case(BillingPeriodEnum::Monthly, "2024-01-01", "2024-01-01", 1, 0)]
        // Within first cycle → 0 elapsed
        #[case(BillingPeriodEnum::Monthly, "2024-01-01", "2024-01-15", 1, 0)]
        // Exactly on first boundary → 1 elapsed (cycle [Jan1,Feb1) is complete)
        #[case(BillingPeriodEnum::Monthly, "2024-01-01", "2024-02-01", 1, 1)]
        // One day into second cycle → 1 elapsed
        #[case(BillingPeriodEnum::Monthly, "2024-01-01", "2024-02-02", 1, 1)]
        // Multiple months
        #[case(BillingPeriodEnum::Monthly, "2024-01-01", "2024-06-15", 1, 5)]
        // Exactly on the 6th boundary
        #[case(BillingPeriodEnum::Monthly, "2024-01-01", "2024-07-01", 1, 6)]
        // Multi-year span (3 years monthly = 36 cycles)
        #[case(BillingPeriodEnum::Monthly, "2021-01-01", "2024-01-01", 1, 36)]
        #[case(BillingPeriodEnum::Monthly, "2021-01-01", "2024-01-15", 1, 36)]
        // target before start → 0
        #[case(BillingPeriodEnum::Monthly, "2024-06-01", "2024-01-01", 1, 0)]
        // Partial first period: start=Jan 1, billing_day=10
        // First period: [Jan1, Jan10), then full months from Jan10
        // target=Jan 5 → still in partial first period → 0
        #[case(BillingPeriodEnum::Monthly, "2024-01-01", "2024-01-05", 10, 0)]
        // target=Jan 10 → partial period [Jan1,Jan10) complete → 1
        #[case(BillingPeriodEnum::Monthly, "2024-01-01", "2024-01-10", 10, 1)]
        // target=Feb 10 → partial + [Jan10,Feb10) complete → 2
        #[case(BillingPeriodEnum::Monthly, "2024-01-01", "2024-02-10", 10, 2)]
        // target=Mar 15 → partial + [Jan10,Feb10) + [Feb10,Mar10) complete → 3
        #[case(BillingPeriodEnum::Monthly, "2024-01-01", "2024-03-15", 10, 3)]
        // Quarterly
        #[case(BillingPeriodEnum::Quarterly, "2024-01-01", "2024-01-15", 1, 0)]
        #[case(BillingPeriodEnum::Quarterly, "2024-01-01", "2024-04-01", 1, 1)]
        #[case(BillingPeriodEnum::Quarterly, "2024-01-01", "2024-07-01", 1, 2)]
        #[case(BillingPeriodEnum::Quarterly, "2024-01-01", "2024-07-15", 1, 2)]
        // Quarterly partial first: start=Jan 10, billing_day=1
        // First period: [Jan10, Apr1), then [Apr1,Jul1), [Jul1,Oct1)...
        #[case(BillingPeriodEnum::Quarterly, "2024-01-10", "2024-04-01", 1, 1)]
        #[case(BillingPeriodEnum::Quarterly, "2024-01-10", "2024-07-15", 1, 2)]
        // Annual
        #[case(BillingPeriodEnum::Annual, "2021-01-01", "2021-06-15", 1, 0)]
        #[case(BillingPeriodEnum::Annual, "2021-01-01", "2022-01-01", 1, 1)]
        #[case(BillingPeriodEnum::Annual, "2021-01-01", "2024-06-15", 1, 3)]
        // Annual partial first: start=Jan 10, billing_day=1
        // First period: [Jan10, Jan1-2022) → wait, day(10) >= anchor(1) so NOT partial.
        // Actually with billing_day=1 and start=Jan 10: started_after_billing_day=true → full period
        // [Jan10, Feb10-next-year?) No: add_months(Jan10, 12, 1) = Jan1+1year clamped to billing_day=1 = 2022-01-01? No...
        // add_months_at_billing_day: date.checked_add_months(12).and_then(|d| d.with_day(min(days_in_month, 1)))
        // Jan10 + 12 months = Jan10-2022.with_day(1) = Jan1-2022
        // So period = [Jan10-2021, Jan1-2022). Then [Jan1-2022, Jan1-2023).
        #[case(BillingPeriodEnum::Annual, "2021-01-10", "2022-01-01", 1, 1)]
        #[case(BillingPeriodEnum::Annual, "2021-01-10", "2023-01-01", 1, 2)]
        // billing_day=15, Monthly: start=Jan 1 (before anchor), partial first = [Jan1,Jan15)
        #[case(BillingPeriodEnum::Monthly, "2024-01-01", "2024-01-14", 15, 0)]
        #[case(BillingPeriodEnum::Monthly, "2024-01-01", "2024-01-15", 15, 1)]
        #[case(BillingPeriodEnum::Monthly, "2024-01-01", "2024-02-15", 15, 2)]
        // billing_day=31 with short months: start=Jan 31
        // First period (day=31 >= anchor=31): [Jan31, Feb28/29). Feb has 29 days in 2024 (leap).
        // add_months(Jan31, 1, 31) = Feb29 (min(29,31)=29). Period = [Jan31, Feb29).
        #[case(BillingPeriodEnum::Monthly, "2024-01-31", "2024-02-15", 31, 0)]
        #[case(BillingPeriodEnum::Monthly, "2024-01-31", "2024-02-29", 31, 1)]
        // Next: [Feb29, Mar31). add_months(Feb29, 1, 31) = Mar31.
        #[case(BillingPeriodEnum::Monthly, "2024-01-31", "2024-03-31", 31, 2)]
        fn test_calculate_elapsed_cycles(
            #[case] billing_period: BillingPeriodEnum,
            #[case] billing_start: NaiveDate,
            #[case] target: NaiveDate,
            #[case] billing_day: u32,
            #[case] expected: u32,
        ) {
            let result =
                calculate_elapsed_cycles(billing_start, target, &billing_period, billing_day);
            assert_eq!(
                result, expected,
                "elapsed_cycles({billing_start}, {target}, {billing_period:?}, day={billing_day}): got {result}, expected {expected}"
            );
        }

        // ─── Tests for find_period_containing_date ───

        #[rstest]
        // Same day → first period
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-01",
            "2024-01-01",
            1,
            "2024-01-01",
            "2024-02-01"
        )]
        // Within first cycle
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-01",
            "2024-01-15",
            1,
            "2024-01-01",
            "2024-02-01"
        )]
        // Exactly on boundary → next period (half-open: Feb1 belongs to [Feb1,Mar1))
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-01",
            "2024-02-01",
            1,
            "2024-02-01",
            "2024-03-01"
        )]
        // One day into second cycle
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-01",
            "2024-02-02",
            1,
            "2024-02-01",
            "2024-03-01"
        )]
        // Mid-year
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-01",
            "2024-06-15",
            1,
            "2024-06-01",
            "2024-07-01"
        )]
        // Multi-year (monthly → period is one month, not one year)
        #[case(
            BillingPeriodEnum::Monthly,
            "2021-01-01",
            "2024-01-01",
            1,
            "2024-01-01",
            "2024-02-01"
        )]
        // target before start → returns first period
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-06-01",
            "2024-01-01",
            1,
            "2024-06-01",
            "2024-07-01"
        )]
        // Partial first period: start=Jan 1, billing_day=10
        // First period: [Jan1, Jan10), then [Jan10, Feb10), [Feb10, Mar10)...
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-01",
            "2024-01-05",
            10,
            "2024-01-01",
            "2024-01-10"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-01",
            "2024-01-10",
            10,
            "2024-01-10",
            "2024-02-10"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-01",
            "2024-01-20",
            10,
            "2024-01-10",
            "2024-02-10"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-01",
            "2024-02-10",
            10,
            "2024-02-10",
            "2024-03-10"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-01",
            "2024-03-15",
            10,
            "2024-03-10",
            "2024-04-10"
        )]
        // Quarterly
        #[case(
            BillingPeriodEnum::Quarterly,
            "2024-01-01",
            "2024-02-15",
            1,
            "2024-01-01",
            "2024-04-01"
        )]
        #[case(
            BillingPeriodEnum::Quarterly,
            "2024-01-01",
            "2024-04-01",
            1,
            "2024-04-01",
            "2024-07-01"
        )]
        #[case(
            BillingPeriodEnum::Quarterly,
            "2024-01-01",
            "2024-07-15",
            1,
            "2024-07-01",
            "2024-10-01"
        )]
        // Annual
        #[case(
            BillingPeriodEnum::Annual,
            "2021-01-01",
            "2021-06-15",
            1,
            "2021-01-01",
            "2022-01-01"
        )]
        #[case(
            BillingPeriodEnum::Annual,
            "2021-01-01",
            "2022-01-01",
            1,
            "2022-01-01",
            "2023-01-01"
        )]
        #[case(
            BillingPeriodEnum::Annual,
            "2021-01-01",
            "2024-06-15",
            1,
            "2024-01-01",
            "2025-01-01"
        )]
        // billing_day=15, partial first: [Jan1,Jan15), then [Jan15,Feb15), ...
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-01",
            "2024-01-14",
            15,
            "2024-01-01",
            "2024-01-15"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-01",
            "2024-01-15",
            15,
            "2024-01-15",
            "2024-02-15"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-01",
            "2024-02-15",
            15,
            "2024-02-15",
            "2024-03-15"
        )]
        // billing_day=31 with short months (2024 is leap year)
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-31",
            "2024-02-15",
            31,
            "2024-01-31",
            "2024-02-29"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-31",
            "2024-02-29",
            31,
            "2024-02-29",
            "2024-03-31"
        )]
        #[case(
            BillingPeriodEnum::Monthly,
            "2024-01-31",
            "2024-03-31",
            31,
            "2024-03-31",
            "2024-04-30"
        )]
        fn test_find_period_containing_date(
            #[case] billing_period: BillingPeriodEnum,
            #[case] billing_start: NaiveDate,
            #[case] target: NaiveDate,
            #[case] billing_day: u32,
            #[case] expected_start: NaiveDate,
            #[case] expected_end: NaiveDate,
        ) {
            let Period { start, end } =
                find_period_containing_date(billing_start, target, &billing_period, billing_day);
            assert_eq!(
                start, expected_start,
                "find_period({billing_start}, {target}, {billing_period:?}, day={billing_day}): start got {start}, expected {expected_start}"
            );
            assert_eq!(
                end, expected_end,
                "find_period({billing_start}, {target}, {billing_period:?}, day={billing_day}): end got {end}, expected {expected_end}"
            );
        }

        // ─── Cross-validation: elapsed_cycles and find_period must agree ───

        #[rstest]
        #[case(BillingPeriodEnum::Monthly, "2021-01-01", "2024-06-15", 1)]
        #[case(BillingPeriodEnum::Monthly, "2023-03-15", "2024-06-01", 15)]
        #[case(BillingPeriodEnum::Quarterly, "2022-01-01", "2024-10-01", 1)]
        #[case(BillingPeriodEnum::Annual, "2020-01-01", "2024-06-15", 1)]
        #[case(BillingPeriodEnum::Monthly, "2024-01-01", "2024-01-31", 10)]
        fn test_elapsed_and_period_consistency(
            #[case] billing_period: BillingPeriodEnum,
            #[case] billing_start: NaiveDate,
            #[case] target: NaiveDate,
            #[case] billing_day: u32,
        ) {
            let elapsed =
                calculate_elapsed_cycles(billing_start, target, &billing_period, billing_day);
            let period =
                find_period_containing_date(billing_start, target, &billing_period, billing_day);

            // target must be within the returned period
            assert!(
                target >= period.start && target < period.end,
                "target {target} not in [{}, {})",
                period.start,
                period.end
            );

            // The elapsed count should match: if we advance `elapsed` periods from start,
            // we should reach the containing period's start
            let mut current_start = billing_start;
            for _ in 0..elapsed {
                let p = calculate_advance_period_range(
                    current_start,
                    billing_day,
                    current_start == billing_start,
                    &billing_period,
                );
                current_start = p.end;
            }
            assert_eq!(
                current_start, period.start,
                "After {elapsed} cycles from {billing_start}, expected to be at {}, but got {current_start}",
                period.start
            );
        }

        fn p(start: &str, end: &str) -> Period {
            Period {
                start: start.parse().unwrap(),
                end: end.parse().unwrap(),
            }
        }

        #[test]
        fn proration_equal_periods_return_none() {
            let full = p("2025-01-01", "2025-02-01");
            assert_eq!(calculate_proration_factor(&full, &full), None);
        }

        #[test]
        fn proration_monthly_half_period() {
            let full = p("2025-01-01", "2025-02-01");
            let partial = p("2025-01-17", "2025-02-01");
            let factor = calculate_proration_factor(&partial, &full).unwrap();
            assert_eq!(factor, crate::services::proration::DayFraction::new(15, 31));
        }

        #[test]
        fn proration_empty_partial_is_zero() {
            // Zero-day partial (e.g. addon purchase at period end) should bill 0, not full.
            assert_eq!(
                calculate_proration_factor(
                    &p("2025-01-15", "2025-01-15"),
                    &p("2025-01-01", "2025-02-01")
                ),
                Some(crate::services::proration::DayFraction::new(0, 31))
            );
        }

        #[test]
        fn proration_empty_full_returns_none() {
            assert_eq!(
                calculate_proration_factor(
                    &p("2025-01-15", "2025-01-20"),
                    &p("2025-01-01", "2025-01-01")
                ),
                None
            );
        }

        #[test]
        fn proration_partial_exceeds_full_returns_none() {
            assert_eq!(
                calculate_proration_factor(
                    &p("2025-01-01", "2025-03-01"),
                    &p("2025-01-01", "2025-02-01")
                ),
                None
            );
        }
    }

    /// Monthly to quarterly at Feb 1. On grid, May 1 bills [May 1, Aug 1) and settles [Feb 1, May
    /// 1). Left ending Mar 1 by an earlier release, Mar 1 bills nothing and May 1 the same as on
    /// grid. Quarterly to monthly at Apr 1, left ending Jul 1: Jul 1 catches up month by month.
    #[test]
    fn a_cadence_run_bills_each_period_once_from_the_first_renewal_on_it() {
        use super::{CadenceRun, FeeRun};
        let fee = |run: CadenceRun, months| FeeRun::of_cadence_run(&run, months, 1).unwrap();
        let d = |m, day| NaiveDate::from_ymd_opt(2024, m, day).unwrap();
        let p = |a: NaiveDate, b: NaiveDate| Period { start: a, end: b };
        let run = |previous| CadenceRun {
            switched_on: d(2, 1),
            origin: d(2, 1),
            previous_renewal: previous,
        };
        assert_eq!(
            fee(run(d(1, 1)), 3).advance_periods(d(2, 1), d(2, 1)),
            vec![p(d(2, 1), d(5, 1))]
        );
        assert_eq!(
            fee(run(d(1, 1)), 3).arrear_windows(d(2, 1), false),
            vec![(
                p(d(1, 1), d(2, 1)),
                p(NaiveDate::from_ymd_opt(2023, 11, 1).unwrap(), d(2, 1))
            )]
        );
        assert_eq!(
            fee(run(d(2, 1)), 3)
                .period(d(2, 1))
                .map(|period| period.end),
            Some(d(5, 1))
        );
        assert_eq!(
            fee(run(d(2, 1)), 3).advance_periods(d(5, 1), d(2, 1)),
            vec![p(d(5, 1), d(8, 1))]
        );
        assert_eq!(
            fee(run(d(2, 1)), 3).arrear_windows(d(5, 1), false),
            vec![(p(d(2, 1), d(5, 1)), p(d(2, 1), d(5, 1)))]
        );

        assert!(
            fee(run(d(2, 1)), 3)
                .advance_periods(d(3, 1), d(2, 1))
                .is_empty()
        );
        assert!(
            fee(run(d(2, 1)), 3)
                .arrear_windows(d(3, 1), false)
                .is_empty()
        );
        assert_eq!(
            fee(run(d(2, 1)), 3)
                .period(d(3, 1))
                .map(|period| period.end),
            Some(d(5, 1))
        );
        assert_eq!(
            fee(run(d(3, 1)), 3).arrear_windows(d(5, 1), false),
            vec![(p(d(2, 1), d(5, 1)), p(d(2, 1), d(5, 1)))]
        );

        let monthly = CadenceRun {
            switched_on: d(4, 1),
            origin: d(4, 1),
            previous_renewal: d(4, 1),
        };
        assert_eq!(
            fee(monthly, 1).advance_periods(d(7, 1), d(4, 1)),
            vec![
                p(d(5, 1), d(6, 1)),
                p(d(6, 1), d(7, 1)),
                p(d(7, 1), d(8, 1))
            ]
        );
        assert_eq!(
            fee(monthly, 1).arrear_windows(d(7, 1), false),
            vec![
                (p(d(4, 1), d(5, 1)), p(d(4, 1), d(5, 1))),
                (p(d(5, 1), d(6, 1)), p(d(5, 1), d(6, 1))),
                (p(d(6, 1), d(7, 1)), p(d(6, 1), d(7, 1))),
            ]
        );
        assert_eq!(
            fee(monthly, 1).period(d(7, 1)).map(|period| period.end),
            Some(d(8, 1))
        );
    }

    /// Monthly to quarterly at Feb 1, which an earlier release renewed on Mar 1 for [Mar 1, Jun 1):
    /// the grid runs from Mar 1. Jun 1 bills [Jun 1, Sep 1) and settles [Mar 1, Jun 1); an early
    /// end on Jul 15 settles from Jun 1; that release's Mar 1 draft settles February.
    #[test]
    fn a_grid_an_earlier_release_renewed_on_bills_each_period_once() {
        use super::{CadenceRun, FeeRun};
        let fee = |run: CadenceRun, months| FeeRun::of_cadence_run(&run, months, 1).unwrap();
        let d = |m, day| NaiveDate::from_ymd_opt(2024, m, day).unwrap();
        let p = |a: NaiveDate, b: NaiveDate| Period { start: a, end: b };
        let run = |previous| CadenceRun {
            switched_on: d(2, 1),
            origin: d(3, 1),
            previous_renewal: previous,
        };
        assert_eq!(
            fee(run(d(3, 1)), 3).advance_periods(d(6, 1), d(2, 1)),
            vec![p(d(6, 1), d(9, 1))]
        );
        assert_eq!(
            fee(run(d(3, 1)), 3).arrear_windows(d(6, 1), false),
            vec![(p(d(3, 1), d(6, 1)), p(d(3, 1), d(6, 1)))]
        );
        assert_eq!(
            fee(run(d(3, 1)), 3)
                .period(d(6, 1))
                .map(|period| period.end),
            Some(d(9, 1))
        );
        assert_eq!(
            fee(run(d(6, 1)), 3).arrear_windows(d(7, 15), true),
            vec![(p(d(6, 1), d(7, 15)), p(d(6, 1), d(9, 1)))]
        );
        assert_eq!(
            fee(run(d(9, 1)), 3).advance_periods(d(12, 1), d(2, 1)),
            vec![p(d(12, 1), NaiveDate::from_ymd_opt(2025, 3, 1).unwrap())]
        );
        assert_eq!(
            fee(run(d(9, 1)), 3).arrear_windows(d(12, 1), false),
            vec![(p(d(9, 1), d(12, 1)), p(d(9, 1), d(12, 1)))]
        );
        assert_eq!(
            fee(run(d(2, 1)), 3).arrear_windows(d(3, 1), false),
            vec![(
                p(d(2, 1), d(3, 1)),
                p(NaiveDate::from_ymd_opt(2023, 12, 1).unwrap(), d(3, 1))
            )]
        );
    }
}

#[cfg(test)]
mod allowance_tests {
    use super::{FeeRun, component_periods_on};
    use crate::services::proration::DayFraction;
    use chrono::NaiveDate;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, m, day).unwrap()
    }

    /// The share of its period an arrear window gives a capacity's allowance, as the invoice
    /// prices it: 16 of 31 days cut by a fixed term's end or a cancellation, or for a fee started
    /// on Jan 16; 22 of 31 days in a first partial period from Jan 10.
    /// Rules: R-002
    #[test]
    fn an_arrear_window_shares_a_capacitys_allowance_as_its_fee_is_prorated() {
        let share = |start: NaiveDate, invoice: NaiveDate, from, term_end, completed| {
            let run = FeeRun::from_start((start, start), 1, 1, invoice).unwrap();
            component_periods_on(&run, invoice, from, None, term_end, completed, false)
                .into_iter()
                .find(|periods| periods.arrear.is_some())
                .map(|periods| periods.arrear_share)
        };
        let sixteen = Some(Some(DayFraction::new(16, 31)));
        assert_eq!(
            share(d(1, 1), d(1, 17), None, Some(d(1, 17)), true),
            sixteen
        );
        assert_eq!(share(d(1, 1), d(1, 17), None, None, true), sixteen);
        assert_eq!(
            share(d(1, 1), d(2, 1), Some(d(1, 16)), None, false),
            sixteen
        );
        assert_eq!(
            share(d(1, 10), d(2, 1), None, None, false),
            Some(Some(DayFraction::new(22, 31)))
        );
        assert_eq!(share(d(1, 1), d(2, 1), None, None, false), Some(None));
    }
}
