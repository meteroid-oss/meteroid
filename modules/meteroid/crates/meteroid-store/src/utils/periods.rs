use common_utils::date::NaiveDateExt;

use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
use crate::domain::{ComponentPeriods, Period};
use chrono::{Datelike, Months, NaiveDate};

/**
 * For a given invoice date, this checks if a component with this period should be billed in that invoice and what are the related arrear/advance periods
 */
pub fn calculate_component_period_for_invoice_date(
    invoice_date: NaiveDate,
    subscription_billing_period: &BillingPeriodEnum,
    component_billing_period: &SubscriptionFeeBillingPeriod,
    billing_start_or_resume_date: NaiveDate,
    cycle_index: u32,
    billing_day: u32,
    is_completed: bool,
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
            advance: Some(Period {
                start: invoice_date,
                end: invoice_date,
            }),
            arrear: None,
        }),
        Some(billing_period) => {
            let (advance_period, proration_factor) = if is_completed {
                (None, None)
            } else {
                let advance_period = calculate_advance_period_range(
                    invoice_date,
                    billing_day,
                    billing_start_or_resume_date == invoice_date,
                    &billing_period,
                );
                let proration_factor = if cycle_index == 0 {
                    calculate_proration_factor(&advance_period)
                } else {
                    None
                };
                (Some(advance_period), proration_factor)
            };

            let arrear_period = if cycle_index == 0 {
                None
            } else {
                Some(calculate_arrear_period_range(
                    invoice_date,
                    billing_start_or_resume_date,
                    billing_day,
                    &billing_period,
                ))
            };

            Some(ComponentPeriods {
                proration_factor,
                advance: advance_period,
                arrear: arrear_period,
            })
        }
    }
}

pub fn calculate_proration_factor(period: &Period) -> Option<f64> {
    let days_in_period = period.end.signed_duration_since(period.start).num_days() as u64; // +1 ?
    let days_in_month_from = u64::from(period.start.days_in_month());
    let days_in_month_to = u64::from(period.end.days_in_month());

    // if from is end of month and from.day <= to.day. Ex: 2023-02-28 -> 2023-03-28+
    if period.start.day() == days_in_month_from as u32 && period.end.day() >= period.start.day() {
        return None;
    }

    if days_in_period >= days_in_month_from {
        return None;
    }

    // if to is end of month and from.day >= to.day. Ex: 2023-01-28+ -> 2023-02-28
    if period.end.day() == days_in_month_to as u32 && period.start.day() >= period.end.day() {
        return None;
    }

    let proration_factor = days_in_period as f64 / days_in_month_from as f64;

    Some(proration_factor.clamp(0.0, 1.0))
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
    let months_per_period = billing_period.as_months();

    // For arrear billing, we're billing for a period that has already ended
    let period_end = invoice_date;

    let period_start = subtract_months_at_billing_day(period_end, months_per_period, billing_day)
        .expect("Failed to calculate arrear period start")
        .max(billing_start_or_resume_date);

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

fn subtract_months_at_billing_day(
    date: NaiveDate,
    months_to_subtract: u32,
    billing_day: u32,
) -> Option<NaiveDate> {
    date.checked_sub_months(Months::new(months_to_subtract))
        .and_then(|d| d.with_day(d.days_in_month().min(billing_day)))
}

#[cfg(test)]
mod test {
    use super::{
        Period, calculate_advance_period_range, calculate_arrear_period_range,
        calculate_elapsed_cycles, find_period_containing_date,
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
    }
}
