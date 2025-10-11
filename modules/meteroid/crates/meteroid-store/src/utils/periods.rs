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

fn calculate_proration_factor(period: &Period) -> Option<f64> {
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

    Some(proration_factor)
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
    use super::{Period, calculate_advance_period_range, calculate_arrear_period_range};
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
    }
}
