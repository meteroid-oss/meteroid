use common_utils::date::NaiveDateExt;

use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
use crate::domain::{ComponentPeriods, Period};
use crate::services::proration::DayFraction;
use chrono::{Datelike, Months, NaiveDate};

/**
 * For a given invoice date, this checks if a component with this period should be billed in that invoice and what are the related arrear/advance periods
 */
#[allow(clippy::too_many_arguments)]
pub fn calculate_component_period_for_invoice_date(
    invoice_date: NaiveDate,
    subscription_billing_period: &BillingPeriodEnum,
    component_billing_period: &SubscriptionFeeBillingPeriod,
    billing_start_or_resume_date: NaiveDate,
    cycle_index: u32,
    billing_day: u32,
    is_completed: bool,
    is_cancellation: bool,
) -> Option<ComponentPeriods> {
    if !applies_this_period(
        cycle_index,
        subscription_billing_period,
        component_billing_period,
    ) {
        return None;
    }

    let billing_period = component_billing_period.as_billing_period_opt();

    match billing_period {
        None => Some(ComponentPeriods {
            proration_factor: None,
            proration_days: None,
            arrear_proration_factor: None,
            advance: Some(Period {
                start: invoice_date,
                end: invoice_date,
            }),
            arrear: None,
        }),
        Some(billing_period) => {
            let (advance_period, proration_factor, proration_days) = if is_completed {
                (None, None, None)
            } else {
                let advance_period = calculate_advance_period_range(
                    invoice_date,
                    billing_day,
                    billing_start_or_resume_date == invoice_date,
                    &billing_period,
                );
                let full_period = if cycle_index == 0 {
                    subtract_months_at_billing_day(
                        advance_period.end,
                        billing_period.as_months(),
                        billing_day,
                    )
                    .map(|full_start| Period {
                        start: full_start,
                        end: advance_period.end,
                    })
                } else {
                    None
                };
                let proration_factor = full_period
                    .as_ref()
                    .and_then(|full| calculate_proration_factor(&advance_period, full));
                let proration_days =
                    full_period
                        .filter(|_| proration_factor.is_some())
                        .map(|full| {
                            (
                                (advance_period.end - advance_period.start).num_days(),
                                (full.end - full.start).num_days(),
                            )
                        });
                (Some(advance_period), proration_factor, proration_days)
            };

            // Cycle 0 normally has no elapsed period to bill in arrears — except when the
            // subscription settles early (`is_completed`: termination, pause, completion), where
            // the partial first period [billing_start, invoice_date] is real and its usage would
            // otherwise never be billed. Mirrors the cycle>0 mid-period cancel, which also bills
            // arrears up to the settlement date.
            let arrear_period = if cycle_index == 0 && !is_completed {
                None
            } else {
                Some(calculate_arrear_period_range(
                    invoice_date,
                    billing_start_or_resume_date,
                    billing_day,
                    &billing_period,
                ))
            };

            // A cancellation clips the arrear window to the termination date; prorate the fixed-rate
            // fee to the elapsed fraction, symmetric to the advance refund. Boundary settlements
            // (renewal, completion, end-of-period cancel) cover the full period, so this stays None.
            let arrear_proration_factor = if is_cancellation {
                arrear_period.as_ref().and_then(|arrear| {
                    let nominal_end = find_period_containing_date(
                        billing_start_or_resume_date,
                        invoice_date.pred_opt().unwrap_or(invoice_date),
                        &billing_period,
                        billing_day,
                    )
                    .end;
                    calculate_proration_factor(
                        arrear,
                        &Period {
                            start: arrear.start,
                            end: nominal_end,
                        },
                    )
                })
            } else {
                None
            };

            Some(ComponentPeriods {
                proration_factor,
                proration_days,
                arrear_proration_factor,
                advance: advance_period,
                arrear: arrear_period,
            })
        }
    }
}

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

fn applies_this_period(
    cycle_index: u32,
    subscription_billing_period: &BillingPeriodEnum,
    component_billing_period: &SubscriptionFeeBillingPeriod,
) -> bool {
    let months_elapsed = (subscription_billing_period.as_months() * cycle_index) as i32;
    months_elapsed % component_billing_period.as_months() == 0
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

    /// The windows an invoice dated `invoice_date` settles in arrears on a `months` grid, one per
    /// grid period with the period it prorates against: what no renewal settled yet (from the
    /// earlier of the previous renewal and the previous boundary), on a boundary or when the
    /// subscription settles early. On the first renewal of the run, what ran since the previous
    /// renewal, against the new cadence's period ending there.
    pub fn arrear_windows(
        &self,
        invoice_date: NaiveDate,
        months: u32,
        billing_day: u32,
        settles_early: bool,
    ) -> Vec<(Period, Period)> {
        if invoice_date <= self.origin.min(self.previous_renewal) || months == 0 {
            return vec![];
        }
        if invoice_date == self.origin {
            let Some(nominal_start) =
                subtract_months_at_billing_day(self.origin, months, billing_day)
            else {
                return vec![];
            };
            let start = nominal_start.max(self.previous_renewal);
            if start >= invoice_date {
                return vec![];
            }
            return vec![(
                Period {
                    start,
                    end: invoice_date,
                },
                Period {
                    start: nominal_start,
                    end: invoice_date,
                },
            )];
        }
        if !settles_early && !self.is_boundary(invoice_date, months, billing_day) {
            return vec![];
        }
        let Some(last) = invoice_date
            .pred_opt()
            .and_then(|day| self.period_holding(day, months, billing_day))
        else {
            return vec![];
        };
        let start = self.previous_renewal.min(last.start).max(self.origin);
        let mut windows = Vec::new();
        let mut cursor = start;
        while cursor < invoice_date {
            let Some(grid) = self.period_holding(cursor, months, billing_day) else {
                break;
            };
            let end = grid.end.min(invoice_date);
            windows.push((Period { start: cursor, end }, grid.clone()));
            cursor = end;
        }
        windows
    }

    /// The advance periods of a `months` grid an invoice dated `invoice_date` bills: the one
    /// starting on it, and any started since the previous renewal that no renewal billed.
    pub fn advance_periods(
        &self,
        invoice_date: NaiveDate,
        months: u32,
        billing_day: u32,
        not_before: NaiveDate,
    ) -> Vec<Period> {
        let mut periods = Vec::new();
        if months == 0 {
            return periods;
        }
        let mut n = 0;
        while let (Some(start), Some(end)) = (
            self.boundary(n, months, billing_day),
            self.boundary(n + 1, months, billing_day),
        ) {
            if start > invoice_date {
                break;
            }
            if (start > self.previous_renewal || start == invoice_date) && start >= not_before {
                periods.push(Period { start, end });
            }
            n += 1;
        }
        periods
    }

    /// The end of the grid period after `start` on a `months` grid: a renewal's period end.
    pub fn period_end_after(
        &self,
        start: NaiveDate,
        months: u32,
        billing_day: u32,
    ) -> Option<NaiveDate> {
        self.period_holding(start, months, billing_day)
            .map(|period| period.end)
    }
}

/// The periods a component running on a [`CadenceRun`] bills on an invoice: each arrears window
/// cut to the component's life and prorated against its own grid period, and each advance period.
#[allow(clippy::too_many_arguments)]
pub fn component_periods_in_run(
    run: &CadenceRun,
    invoice_date: NaiveDate,
    component_billing_period: &SubscriptionFeeBillingPeriod,
    billing_day: u32,
    effective_from: Option<NaiveDate>,
    effective_to: Option<NaiveDate>,
    is_completed: bool,
) -> Vec<ComponentPeriods> {
    let Some(cadence) = component_billing_period.as_billing_period_opt() else {
        return vec![];
    };
    let months = cadence.as_months();
    let mut periods: Vec<ComponentPeriods> = Vec::new();
    for (window, nominal) in run.arrear_windows(invoice_date, months, billing_day, is_completed) {
        let start = effective_from.map_or(window.start, |from| from.max(window.start));
        let end = effective_to.map_or(window.end, |to| to.min(window.end));
        if start >= end {
            continue;
        }
        let days = (end - start).num_days();
        let nominal_days = (nominal.end - nominal.start).num_days();
        periods.push(ComponentPeriods {
            proration_factor: None,
            proration_days: None,
            arrear_proration_factor: (days < nominal_days)
                .then(|| DayFraction::new(days, nominal_days)),
            advance: None,
            arrear: Some(Period { start, end }),
        });
    }
    if !is_completed {
        let not_before = effective_from.unwrap_or(run.switched_on);
        for advance in run.advance_periods(invoice_date, months, billing_day, not_before) {
            periods.push(ComponentPeriods {
                proration_factor: None,
                proration_days: None,
                arrear_proration_factor: None,
                advance: Some(advance),
                arrear: None,
            });
        }
    }
    periods
}

#[cfg(test)]
mod test {
    use super::{
        Period, calculate_advance_period_range, calculate_arrear_period_range,
        calculate_component_period_for_invoice_date, calculate_elapsed_cycles,
        calculate_proration_factor, find_period_containing_date,
    };
    use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};

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

        #[test]
        fn first_cycle_quarterly_partial_period_is_prorated() {
            // Regression: quarterly started 2025-10-08 with anchor day=1 must prorate.
            let invoice_date = NaiveDate::from_ymd_opt(2025, 10, 8).unwrap();
            let periods = calculate_component_period_for_invoice_date(
                invoice_date,
                &BillingPeriodEnum::Quarterly,
                &SubscriptionFeeBillingPeriod::Quarterly,
                invoice_date,
                0,
                1,
                false,
                false,
            )
            .expect("should return periods");

            let advance = periods.advance.expect("should have advance");
            assert_eq!(advance.start, NaiveDate::from_ymd_opt(2025, 10, 8).unwrap());
            assert_eq!(advance.end, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());

            let factor = periods.proration_factor.expect("should be prorated");
            assert_eq!(factor, crate::services::proration::DayFraction::new(85, 92));
        }

        #[test]
        fn first_cycle_quarterly_aligned_no_proration() {
            let invoice_date = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
            let periods = calculate_component_period_for_invoice_date(
                invoice_date,
                &BillingPeriodEnum::Quarterly,
                &SubscriptionFeeBillingPeriod::Quarterly,
                invoice_date,
                0,
                1,
                false,
                false,
            )
            .expect("should return periods");

            assert_eq!(periods.proration_factor, None);
        }

        #[test]
        fn first_cycle_annual_partial_period_is_prorated() {
            let invoice_date = NaiveDate::from_ymd_opt(2025, 4, 15).unwrap();
            let periods = calculate_component_period_for_invoice_date(
                invoice_date,
                &BillingPeriodEnum::Annual,
                &SubscriptionFeeBillingPeriod::Annual,
                invoice_date,
                0,
                1,
                false,
                false,
            )
            .expect("should return periods");

            let advance = periods.advance.expect("should have advance");
            assert_eq!(advance.start, NaiveDate::from_ymd_opt(2025, 4, 15).unwrap());
            assert_eq!(advance.end, NaiveDate::from_ymd_opt(2026, 4, 1).unwrap());

            let factor = periods.proration_factor.expect("should be prorated");
            assert_eq!(
                factor,
                crate::services::proration::DayFraction::new(351, 365)
            );
        }

        #[test]
        fn second_cycle_never_prorated() {
            let periods = calculate_component_period_for_invoice_date(
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                &BillingPeriodEnum::Quarterly,
                &SubscriptionFeeBillingPeriod::Quarterly,
                NaiveDate::from_ymd_opt(2025, 10, 8).unwrap(),
                1,
                1,
                false,
                false,
            )
            .expect("should return periods");

            assert_eq!(periods.proration_factor, None);
        }

        #[test]
        fn mid_period_cancellation_prorates_arrears() {
            // Cancelled Mar 15 in period [Mar 1, Apr 1]: arrears prorate to 14 elapsed of 31 days.
            let periods = calculate_component_period_for_invoice_date(
                NaiveDate::from_ymd_opt(2025, 3, 15).unwrap(),
                &BillingPeriodEnum::Monthly,
                &SubscriptionFeeBillingPeriod::Monthly,
                NaiveDate::from_ymd_opt(2025, 3, 1).unwrap(),
                1,
                1,
                true,
                true,
            )
            .expect("should return periods");

            let arrear = periods.arrear.expect("should have arrear");
            assert_eq!(arrear.start, NaiveDate::from_ymd_opt(2025, 3, 1).unwrap());
            assert_eq!(arrear.end, NaiveDate::from_ymd_opt(2025, 3, 15).unwrap());
            let factor = periods
                .arrear_proration_factor
                .expect("arrears prorated on cancellation");
            assert_eq!(factor, crate::services::proration::DayFraction::new(14, 31));
        }

        #[test]
        fn boundary_cancellation_does_not_prorate_arrears() {
            // Cancelled on the boundary (Apr 1): the arrear covers the full period, so no factor.
            let periods = calculate_component_period_for_invoice_date(
                NaiveDate::from_ymd_opt(2025, 4, 1).unwrap(),
                &BillingPeriodEnum::Monthly,
                &SubscriptionFeeBillingPeriod::Monthly,
                NaiveDate::from_ymd_opt(2025, 3, 1).unwrap(),
                1,
                1,
                true,
                true,
            )
            .expect("should return periods");

            assert_eq!(periods.arrear_proration_factor, None);
        }

        #[test]
        fn annual_mid_period_cancellation_prorates_arrears() {
            // Cancelled Jul 1 in annual period [Jan 1 2025, Jan 1 2026]: 181 elapsed of 365 days.
            let periods = calculate_component_period_for_invoice_date(
                NaiveDate::from_ymd_opt(2025, 7, 1).unwrap(),
                &BillingPeriodEnum::Annual,
                &SubscriptionFeeBillingPeriod::Annual,
                NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
                0,
                1,
                true,
                true,
            )
            .expect("should return periods");

            let arrear = periods.arrear.expect("should have arrear");
            assert_eq!(arrear.start, NaiveDate::from_ymd_opt(2025, 1, 1).unwrap());
            assert_eq!(arrear.end, NaiveDate::from_ymd_opt(2025, 7, 1).unwrap());
            let factor = periods
                .arrear_proration_factor
                .expect("arrears prorated on cancellation");
            assert_eq!(
                factor,
                crate::services::proration::DayFraction::new(181, 365)
            );
        }

        #[test]
        fn first_partial_period_cancellation_prorates_by_partial_denominator() {
            // billing_start Jan 10 (mid-month), cycle 0, cancelled Jan 20: the denominator is the
            // partial first period [Jan 10, Feb 1] = 22 days, not the full month, so factor = 10/22.
            let periods = calculate_component_period_for_invoice_date(
                NaiveDate::from_ymd_opt(2025, 1, 20).unwrap(),
                &BillingPeriodEnum::Monthly,
                &SubscriptionFeeBillingPeriod::Monthly,
                NaiveDate::from_ymd_opt(2025, 1, 10).unwrap(),
                0,
                1,
                true,
                true,
            )
            .expect("should return periods");

            let arrear = periods.arrear.expect("should have arrear");
            assert_eq!(arrear.start, NaiveDate::from_ymd_opt(2025, 1, 10).unwrap());
            assert_eq!(arrear.end, NaiveDate::from_ymd_opt(2025, 1, 20).unwrap());
            let factor = periods
                .arrear_proration_factor
                .expect("arrears prorated on cancellation");
            assert_eq!(factor, crate::services::proration::DayFraction::new(10, 22));
        }
    }

    /// Monthly to quarterly at Feb 1. On grid, May 1 bills [May 1, Aug 1) and settles [Feb 1, May
    /// 1). Left ending Mar 1 by an earlier release, Mar 1 bills nothing and May 1 the same as on
    /// grid. Quarterly to monthly at Apr 1, left ending Jul 1: Jul 1 catches up month by month.
    #[test]
    fn a_cadence_run_bills_each_period_once_from_the_first_renewal_on_it() {
        use super::CadenceRun;
        let d = |m, day| NaiveDate::from_ymd_opt(2024, m, day).unwrap();
        let p = |a: NaiveDate, b: NaiveDate| Period { start: a, end: b };
        let run = |previous| CadenceRun {
            switched_on: d(2, 1),
            origin: d(2, 1),
            previous_renewal: previous,
        };
        assert_eq!(
            run(d(1, 1)).advance_periods(d(2, 1), 3, 1, d(2, 1)),
            vec![p(d(2, 1), d(5, 1))]
        );
        assert_eq!(
            run(d(1, 1)).arrear_windows(d(2, 1), 3, 1, false),
            vec![(
                p(d(1, 1), d(2, 1)),
                p(NaiveDate::from_ymd_opt(2023, 11, 1).unwrap(), d(2, 1))
            )]
        );
        assert_eq!(run(d(2, 1)).period_end_after(d(2, 1), 3, 1), Some(d(5, 1)));
        assert_eq!(
            run(d(2, 1)).advance_periods(d(5, 1), 3, 1, d(2, 1)),
            vec![p(d(5, 1), d(8, 1))]
        );
        assert_eq!(
            run(d(2, 1)).arrear_windows(d(5, 1), 3, 1, false),
            vec![(p(d(2, 1), d(5, 1)), p(d(2, 1), d(5, 1)))]
        );

        assert!(
            run(d(2, 1))
                .advance_periods(d(3, 1), 3, 1, d(2, 1))
                .is_empty()
        );
        assert!(run(d(2, 1)).arrear_windows(d(3, 1), 3, 1, false).is_empty());
        assert_eq!(run(d(2, 1)).period_end_after(d(3, 1), 3, 1), Some(d(5, 1)));
        assert_eq!(
            run(d(3, 1)).arrear_windows(d(5, 1), 3, 1, false),
            vec![(p(d(2, 1), d(5, 1)), p(d(2, 1), d(5, 1)))]
        );

        let monthly = CadenceRun {
            switched_on: d(4, 1),
            origin: d(4, 1),
            previous_renewal: d(4, 1),
        };
        assert_eq!(
            monthly.advance_periods(d(7, 1), 1, 1, d(4, 1)),
            vec![
                p(d(5, 1), d(6, 1)),
                p(d(6, 1), d(7, 1)),
                p(d(7, 1), d(8, 1))
            ]
        );
        assert_eq!(
            monthly.arrear_windows(d(7, 1), 1, 1, false),
            vec![
                (p(d(4, 1), d(5, 1)), p(d(4, 1), d(5, 1))),
                (p(d(5, 1), d(6, 1)), p(d(5, 1), d(6, 1))),
                (p(d(6, 1), d(7, 1)), p(d(6, 1), d(7, 1))),
            ]
        );
        assert_eq!(monthly.period_end_after(d(7, 1), 1, 1), Some(d(8, 1)));
    }

    /// Monthly to quarterly at Feb 1, which an earlier release renewed on Mar 1 for [Mar 1, Jun 1):
    /// the grid runs from Mar 1. Jun 1 bills [Jun 1, Sep 1) and settles [Mar 1, Jun 1); an early
    /// end on Jul 15 settles from Jun 1; that release's Mar 1 draft settles February.
    #[test]
    fn a_grid_an_earlier_release_renewed_on_bills_each_period_once() {
        use super::CadenceRun;
        let d = |m, day| NaiveDate::from_ymd_opt(2024, m, day).unwrap();
        let p = |a: NaiveDate, b: NaiveDate| Period { start: a, end: b };
        let run = |previous| CadenceRun {
            switched_on: d(2, 1),
            origin: d(3, 1),
            previous_renewal: previous,
        };
        assert_eq!(
            run(d(3, 1)).advance_periods(d(6, 1), 3, 1, d(2, 1)),
            vec![p(d(6, 1), d(9, 1))]
        );
        assert_eq!(
            run(d(3, 1)).arrear_windows(d(6, 1), 3, 1, false),
            vec![(p(d(3, 1), d(6, 1)), p(d(3, 1), d(6, 1)))]
        );
        assert_eq!(run(d(3, 1)).period_end_after(d(6, 1), 3, 1), Some(d(9, 1)));
        assert_eq!(
            run(d(6, 1)).arrear_windows(d(7, 15), 3, 1, true),
            vec![(p(d(6, 1), d(7, 15)), p(d(6, 1), d(9, 1)))]
        );
        assert_eq!(
            run(d(9, 1)).advance_periods(d(12, 1), 3, 1, d(2, 1)),
            vec![p(d(12, 1), NaiveDate::from_ymd_opt(2025, 3, 1).unwrap())]
        );
        assert_eq!(
            run(d(9, 1)).arrear_windows(d(12, 1), 3, 1, false),
            vec![(p(d(9, 1), d(12, 1)), p(d(9, 1), d(12, 1)))]
        );
        assert_eq!(
            run(d(2, 1)).arrear_windows(d(3, 1), 3, 1, false),
            vec![(
                p(d(2, 1), d(3, 1)),
                p(NaiveDate::from_ymd_opt(2023, 12, 1).unwrap(), d(3, 1))
            )]
        );
    }
}
